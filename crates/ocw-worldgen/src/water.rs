//! Water cleaning (spec §4.8.4): rivers flow monotonically downhill, lakes
//! are level, and the water never lifts or floods a road.
//!
//! The land-use step ([`crate::landuse::add_landuse`]) gives every water
//! vertex the mean terrain height of the water within 50 m. On SRTM that
//! mean follows the noise of the surface model (trees on the banks,
//! bridges), so rivers step up and down and lakes tilt. This pass replaces
//! those levels after the land use is built:
//!
//! - **Rivers** (OSM `waterway=river|canal|…` lines; their direction is the
//!   flow direction) are joined into chains across the way splits, the
//!   natural DEM is sampled along them every [`RIVER_STEP_M`] and fitted by
//!   a non-increasing profile (isotonic regression, pool-adjacent-violators),
//!   so the level never rises downstream. Tributaries end at the level of
//!   the river they flow into.
//! - **Lakes** (water areas that are not river surfaces) get one level: the
//!   median of the DEM under them.
//! - Every water vertex takes the level of the lake it lies in, else of the
//!   nearest river chain; the terrain below is carved to
//!   [`crate::landuse::WATER_DEPTH_M`] under the new level.
//! - **The sea** ([`crate::landuse::sea`], level [`SEA_LEVEL_M`]) keeps its
//!   level: a water vertex at exactly the sea level outside every lake is
//!   sea or its shore, and a river mouth must not lift it.
//! - Tiles share their border vertices; both copies are computed from the
//!   same position and heights, so they stay equal.
//! - Vertices under a road (the road's half width plus a grid-cell
//!   diagonal, bridges and tunnels excepted) are never carved: they get the
//!   road-conformed height back, so a road along a river bank does not float
//!   above a carved hole.

use std::collections::HashMap;

use ocw_world::landuse::LandClass;
use ocw_world::package::WorldPackage;

use crate::areas::{AreaData, inside_rings};
use crate::landuse::{SEA_LEVEL_M, SURFACE_MIN_WIDTH_M, WATER_DEPTH_M, classify, waterway_width};
use crate::osm::Tags;
use crate::projection::LocalProjection;

/// Spacing of the level samples along a river (m).
pub const RIVER_STEP_M: f64 = 10.0;

/// A river, canal or stream line in local metres, in flow direction.
#[derive(Debug, Clone, PartialEq)]
pub struct River {
    pub osm_way: i64,
    pub nodes: Vec<i64>,
    pub points: Vec<[f64; 2]>,
    pub width_m: f64,
    pub name: Option<String>,
}

/// A standing water area (lake, pond, reservoir) in local metres.
#[derive(Debug, Clone, PartialEq)]
pub struct Lake {
    /// OSM way id, or the negative relation id.
    pub osm_id: i64,
    pub rings: Vec<Vec<[f64; 2]>>,
    /// min x, min z, max x, max z.
    pub bbox: [f64; 4],
}

/// The rivers and lakes of an extract.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaterBodies {
    pub rivers: Vec<River>,
    pub lakes: Vec<Lake>,
}

/// Water areas whose surface belongs to a river (its level comes from the
/// river line), not to a lake.
pub fn is_river_surface(tags: &Tags) -> bool {
    let t = |k: &str| tags.get(k).map(String::as_str);
    t("waterway") == Some("riverbank")
        || matches!(
            t("water"),
            Some("river" | "stream" | "canal" | "ditch" | "drain" | "rapids" | "stream_pool")
        )
}

/// Rivers (lines with a drawn width) and lakes of an extract.
pub fn extract_water(data: &AreaData, proj: &LocalProjection) -> WaterBodies {
    let project = |ring: &[(f64, f64)]| -> Vec<[f64; 2]> {
        ring.iter()
            .map(|&(lat, lon)| proj.project(lat, lon))
            .collect()
    };
    let rivers = data
        .waterways()
        .into_iter()
        .filter_map(|(w, pts)| {
            Some(River {
                osm_way: w.id,
                nodes: w.nodes.clone(),
                width_m: waterway_width(&w.tags)?,
                points: project(&pts),
                name: w.tag("name").map(str::to_string),
            })
        })
        .collect();
    let lakes = data
        .areas()
        .into_iter()
        .filter(|a| classify(a.tags) == Some(LandClass::Water) && !is_river_surface(a.tags))
        .map(|a| {
            let rings: Vec<Vec<[f64; 2]>> =
                a.outer.iter().chain(&a.inner).map(|r| project(r)).collect();
            Lake {
                osm_id: a.id,
                bbox: bbox(rings.iter().flatten()),
                rings,
            }
        })
        .collect();
    WaterBodies { rivers, lakes }
}

fn bbox<'a>(pts: impl Iterator<Item = &'a [f64; 2]>) -> [f64; 4] {
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

impl Lake {
    pub fn contains(&self, p: [f64; 2]) -> bool {
        p[0] >= self.bbox[0]
            && p[0] <= self.bbox[2]
            && p[1] >= self.bbox[1]
            && p[1] <= self.bbox[3]
            && inside_rings(p, &self.rings)
    }
}

/// Best non-increasing fit of `v` (least squares; pool adjacent violators).
pub fn non_increasing(v: &[f64]) -> Vec<f64> {
    // Blocks of (mean, count).
    let mut blocks: Vec<(f64, usize)> = Vec::with_capacity(v.len());
    for &x in v {
        blocks.push((x, 1));
        while blocks.len() >= 2 {
            let (b, nb) = blocks[blocks.len() - 1];
            let (a, na) = blocks[blocks.len() - 2];
            if a >= b {
                break;
            }
            blocks.pop();
            let n = na + nb;
            *blocks.last_mut().expect("two blocks") =
                ((a * na as f64 + b * nb as f64) / n as f64, n);
        }
    }
    blocks
        .into_iter()
        .flat_map(|(m, n)| std::iter::repeat_n(m, n))
        .collect()
}

/// Rivers joined across their OSM way splits: a way continues the chain
/// when it starts at the node where the chain ends and nothing else does.
/// Each chain is a list of river indices in flow order.
pub fn river_chains(rivers: &[River]) -> Vec<Vec<usize>> {
    let mut starting: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut ending: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, r) in rivers.iter().enumerate() {
        if let (Some(&a), Some(&b)) = (r.nodes.first(), r.nodes.last()) {
            starting.entry(a).or_default().push(i);
            ending.entry(b).or_default().push(i);
        }
    }
    let mut used = vec![false; rivers.len()];
    let mut chains = Vec::new();
    // Widest first, so the main river forms its chain before its tributaries.
    let mut order: Vec<usize> = (0..rivers.len()).collect();
    order.sort_by(|&a, &b| {
        rivers[b]
            .width_m
            .total_cmp(&rivers[a].width_m)
            .then(a.cmp(&b))
    });
    let continues = |from: usize, to: usize| {
        let (a, b) = (&rivers[from], &rivers[to]);
        a.name == b.name || (a.width_m - b.width_m).abs() < 1e-9
    };
    for &start in &order {
        if used[start] {
            continue;
        }
        // Walk upstream while exactly one way ends where this starts.
        let mut first = start;
        let mut seen = vec![start];
        loop {
            let node = rivers[first].nodes[0];
            let ups: Vec<usize> = ending
                .get(&node)
                .map(|v| {
                    v.iter()
                        .copied()
                        .filter(|&j| !used[j] && continues(j, first))
                        .collect()
                })
                .unwrap_or_default();
            if ups.len() != 1 || seen.contains(&ups[0]) || ending[&node].len() != 1 {
                break;
            }
            first = ups[0];
            seen.push(first);
        }
        let mut chain = vec![first];
        used[first] = true;
        loop {
            let last = *chain.last().expect("non-empty");
            let node = *rivers[last].nodes.last().expect("nodes");
            let next: Vec<usize> = starting
                .get(&node)
                .map(|v| {
                    v.iter()
                        .copied()
                        .filter(|&j| !used[j] && continues(last, j))
                        .collect()
                })
                .unwrap_or_default();
            if next.len() != 1 {
                break;
            }
            used[next[0]] = true;
            chain.push(next[0]);
        }
        chains.push(chain);
    }
    chains
}

/// A river chain's level profile: positions every [`RIVER_STEP_M`] along
/// the line (flow order) and the level there.
#[derive(Debug, Clone, PartialEq)]
pub struct RiverLevels {
    pub points: Vec<[f64; 2]>,
    pub levels: Vec<f64>,
    pub width_m: f64,
    pub osm_way: i64,
}

impl RiverLevels {
    /// Nearest sample within `radius` of `p`: (distance, level).
    pub fn level_near(&self, p: [f64; 2], radius: f64) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        for w in 0..self.points.len().saturating_sub(1) {
            let (a, b) = (self.points[w], self.points[w + 1]);
            let (d, t, _) = crate::spatial::foot_on_segment(p, a, b);
            if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
                let l = self.levels[w] + (self.levels[w + 1] - self.levels[w]) * t;
                best = Some((d, l));
            }
        }
        if best.is_none() && self.points.len() == 1 {
            let d = (p[0] - self.points[0][0]).hypot(p[1] - self.points[0][1]);
            if d <= radius {
                best = Some((d, self.levels[0]));
            }
        }
        best
    }
}

/// Level profiles of the river chains with a water surface (at least
/// [`SURFACE_MIN_WIDTH_M`] wide): DEM along the line, fitted to a
/// non-increasing profile. A chain ending on (or starting from) a river
/// processed before it keeps at least (at most) that river's level there.
pub fn river_levels(
    rivers: &[River],
    ground: &impl Fn([f64; 2]) -> Option<f64>,
) -> Vec<RiverLevels> {
    let mut out: Vec<RiverLevels> = Vec::new();
    for chain in river_chains(rivers) {
        let width = chain.iter().map(|&i| rivers[i].width_m).fold(0.0, f64::max);
        if width < SURFACE_MIN_WIDTH_M {
            continue;
        }
        let mut line: Vec<[f64; 2]> = Vec::new();
        for &i in &chain {
            let skip = usize::from(!line.is_empty());
            line.extend_from_slice(&rivers[i].points[skip..]);
        }
        let points: Vec<[f64; 2]> = crate::profile::resample(&line, RIVER_STEP_M);
        let raw: Vec<Option<f64>> = points.iter().map(|&p| ground(p)).collect();
        let Some(fallback) = raw.iter().flatten().next().copied() else {
            continue;
        };
        let raw: Vec<f64> = raw.iter().map(|v| v.unwrap_or(fallback)).collect();
        let mut levels = non_increasing(&raw);
        // Confluences with rivers already levelled.
        let (first, last) = (points[0], *points.last().expect("non-empty"));
        let tol = 2.0 * RIVER_STEP_M;
        if let Some((_, l)) = out
            .iter()
            .filter_map(|r| r.level_near(last, width.max(r.width_m) / 2.0 + tol))
            .min_by(|a, b| a.0.total_cmp(&b.0))
        {
            for v in &mut levels {
                *v = v.max(l);
            }
        }
        if let Some((_, l)) = out
            .iter()
            .filter_map(|r| r.level_near(first, width.max(r.width_m) / 2.0 + tol))
            .min_by(|a, b| a.0.total_cmp(&b.0))
        {
            for v in &mut levels {
                *v = v.min(l);
            }
        }
        out.push(RiverLevels {
            points,
            levels,
            width_m: width,
            osm_way: rivers[chain[0]].osm_way,
        });
    }
    out
}

/// River and lake levels with bucket indexes for lookups by position.
pub struct WaterLevels<'a> {
    pub rivers: Vec<RiverLevels>,
    lakes: &'a [Lake],
    lake_levels: Vec<Option<f64>>,
    /// (river, segment) per bucket, segments widened by the river's reach.
    river_buckets: HashMap<(i64, i64), Vec<(u32, u32)>>,
    lake_buckets: HashMap<(i64, i64), Vec<u32>>,
    /// How far beyond its half width a river's level reaches (m).
    reach_m: f64,
}

const BUCKET_M: f64 = 100.0;

fn bucket(p: [f64; 2]) -> (i64, i64) {
    (
        (p[0] / BUCKET_M).floor() as i64,
        (p[1] / BUCKET_M).floor() as i64,
    )
}

impl<'a> WaterLevels<'a> {
    pub fn new(
        bodies: &'a WaterBodies,
        spacing: f64,
        ground: &impl Fn([f64; 2]) -> Option<f64>,
    ) -> Self {
        let rivers = river_levels(&bodies.rivers, ground);
        let reach_m = 3.0 * spacing;
        let mut river_buckets: HashMap<(i64, i64), Vec<(u32, u32)>> = HashMap::new();
        for (ri, r) in rivers.iter().enumerate() {
            let m = r.width_m / 2.0 + reach_m;
            for (si, w) in r.points.windows(2).enumerate() {
                let lo = bucket([w[0][0].min(w[1][0]) - m, w[0][1].min(w[1][1]) - m]);
                let hi = bucket([w[0][0].max(w[1][0]) + m, w[0][1].max(w[1][1]) + m]);
                for bx in lo.0..=hi.0 {
                    for bz in lo.1..=hi.1 {
                        river_buckets
                            .entry((bx, bz))
                            .or_default()
                            .push((ri as u32, si as u32));
                    }
                }
            }
        }
        let mut lake_buckets: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (li, l) in bodies.lakes.iter().enumerate() {
            let (lo, hi) = (
                bucket([l.bbox[0], l.bbox[1]]),
                bucket([l.bbox[2], l.bbox[3]]),
            );
            for bx in lo.0..=hi.0 {
                for bz in lo.1..=hi.1 {
                    lake_buckets.entry((bx, bz)).or_default().push(li as u32);
                }
            }
        }
        Self {
            lake_levels: bodies
                .lakes
                .iter()
                .map(|l| lake_level(l, spacing, ground))
                .collect(),
            rivers,
            lakes: &bodies.lakes,
            river_buckets,
            lake_buckets,
            reach_m,
        }
    }

    /// Level of the (first) lake `p` lies in.
    pub fn lake_at(&self, p: [f64; 2]) -> Option<f64> {
        self.lake_buckets.get(&bucket(p))?.iter().find_map(|&li| {
            if self.lakes[li as usize].contains(p) {
                self.lake_levels[li as usize]
            } else {
                None
            }
        })
    }

    /// Level of the nearest river within its half width plus the reach.
    pub fn river_at(&self, p: [f64; 2]) -> Option<f64> {
        let mut best: Option<(f64, f64)> = None;
        for &(ri, si) in self.river_buckets.get(&bucket(p))? {
            let r = &self.rivers[ri as usize];
            let (a, b) = (r.points[si as usize], r.points[si as usize + 1]);
            let (d, t, _) = crate::spatial::foot_on_segment(p, a, b);
            if d <= r.width_m / 2.0 + self.reach_m && best.is_none_or(|(bd, _)| d < bd) {
                let (la, lb) = (r.levels[si as usize], r.levels[si as usize + 1]);
                best = Some((d, la + (lb - la) * t));
            }
        }
        best.map(|b| b.1)
    }

    /// The level at `p`: its lake's, else the nearest river's.
    pub fn level_at(&self, p: [f64; 2]) -> Option<f64> {
        self.lake_at(p).or_else(|| self.river_at(p))
    }

    pub fn lake_count(&self) -> usize {
        self.lake_levels.iter().flatten().count()
    }
}

/// What [`level_water`] changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaterStats {
    pub river_chains: usize,
    pub lakes: usize,
    /// Water vertices that took a river or lake level.
    pub relevelled: usize,
    /// Vertices under roads given back their road-conformed height.
    pub restored_under_roads: usize,
    /// Largest change of a level (m).
    pub max_change_m: f64,
}

/// Re-levels the water of a package with land use (see the module docs).
/// `ground` is the natural DEM, `road` the road-conformed terrain height at
/// a point if it lies under a road (none elsewhere).
pub fn level_water(
    pkg: &mut WorldPackage,
    bodies: &WaterBodies,
    ground: &impl Fn([f64; 2]) -> Option<f64>,
    road: &(impl Fn([f64; 2]) -> Option<f64> + Sync),
) -> WaterStats {
    let levels = WaterLevels::new(bodies, pkg.spacing_m(), ground);
    let mut stats = WaterStats {
        river_chains: levels.rivers.len(),
        lakes: levels.lake_count(),
        ..Default::default()
    };
    let size = f64::from(pkg.tile_size_m);
    let n = pkg.samples as usize;
    let sp = pkg.spacing_m();
    let Some(lu) = pkg.landuse.as_mut() else {
        return stats;
    };
    for (key, lt) in lu.tiles.iter_mut() {
        if lt.water.is_empty() {
            continue;
        }
        let Some(tile) = pkg.tiles.get_mut(key) else {
            continue;
        };
        let o = key.origin(size);
        for (idx, level) in lt.water.iter_mut() {
            let (i, k) = (*idx as usize % n, *idx as usize / n);
            let p = [o[0] + i as f64 * sp, o[1] + k as f64 * sp];
            let old = f64::from(*level);
            let h = f64::from(tile.heights[*idx as usize]);
            let in_water = h <= old - WATER_DEPTH_M + 1e-3;
            // The sea and its shore keep their fixed level (a lake at 0 m,
            // e.g. a salt pond, is still levelled as a lake).
            let sea = *level == SEA_LEVEL_M as f32 && levels.lake_at(p).is_none();
            // A rim vertex (next to the water, outside its outline) takes
            // the level of the water beside it.
            let new = (!sea).then(|| levels.level_at(p)).flatten().or_else(|| {
                if sea {
                    return None;
                }
                [
                    (-1.0, 0.0),
                    (1.0, 0.0),
                    (0.0, -1.0),
                    (0.0, 1.0),
                    (-1.0, -1.0),
                    (1.0, 1.0),
                    (-1.0, 1.0),
                    (1.0, -1.0),
                ]
                .iter()
                .find_map(|&(dx, dz)| levels.level_at([p[0] + dx * sp, p[1] + dz * sp]))
            });
            if let Some(road_h) = road(p) {
                // Under a road: the road-conformed height, never a hole.
                if in_water || h < road_h - 1e-3 {
                    tile.heights[*idx as usize] = road_h as f32;
                    stats.restored_under_roads += 1;
                }
            } else if let Some(l) = new
                && in_water
            {
                tile.heights[*idx as usize] = (l - WATER_DEPTH_M) as f32;
            }
            if let Some(l) = new {
                stats.relevelled += 1;
                stats.max_change_m = stats.max_change_m.max((l - old).abs());
                *level = l as f32;
            }
        }
    }
    stats
}

/// One level for a lake: the median DEM height at the grid points inside it.
fn lake_level(lake: &Lake, spacing: f64, ground: &impl Fn([f64; 2]) -> Option<f64>) -> Option<f64> {
    let mut hs = Vec::new();
    let b = lake.bbox;
    let mut z = (b[1] / spacing).ceil() * spacing;
    while z <= b[3] {
        let mut x = (b[0] / spacing).ceil() * spacing;
        while x <= b[2] {
            if lake.contains([x, z])
                && let Some(h) = ground([x, z])
            {
                hs.push(h);
            }
            x += spacing;
        }
        z += spacing;
    }
    if hs.is_empty() {
        // Smaller than a grid cell: its first point.
        return lake
            .rings
            .first()
            .and_then(|r| r.first())
            .and_then(|&p| ground(p));
    }
    hs.sort_by(f64::total_cmp);
    Some(hs[hs.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::landuse::{LandUse, LandUseTile};
    use ocw_world::package::{Tile, TileKey};

    fn river(id: i64, nodes: Vec<i64>, points: Vec<[f64; 2]>, width: f64, name: &str) -> River {
        River {
            osm_way: id,
            nodes,
            points,
            width_m: width,
            name: Some(name.into()),
        }
    }

    #[test]
    fn isotonic_fit_never_rises() {
        assert_eq!(
            non_increasing(&[5.0, 4.0, 4.5, 3.0]),
            vec![5.0, 4.25, 4.25, 3.0]
        );
        assert_eq!(non_increasing(&[1.0, 2.0, 3.0]), vec![2.0, 2.0, 2.0]);
        let noisy: Vec<f64> = (0..100)
            .map(|i| 500.0 - i as f64 * 0.1 + ((i * 7) % 5) as f64 * 0.8)
            .collect();
        let fit = non_increasing(&noisy);
        assert!(fit.windows(2).all(|w| w[1] <= w[0] + 1e-12));
        // Least squares keeps the mean.
        let (a, b): (f64, f64) = (noisy.iter().sum(), fit.iter().sum());
        assert!((a - b).abs() < 1e-6);
    }

    #[test]
    fn river_ways_are_chained_in_flow_order() {
        let rs = vec![
            river(
                2,
                vec![2, 3],
                vec![[100.0, 0.0], [200.0, 0.0]],
                20.0,
                "Rhein",
            ),
            river(1, vec![1, 2], vec![[0.0, 0.0], [100.0, 0.0]], 20.0, "Rhein"),
            river(
                3,
                vec![9, 3],
                vec![[200.0, -100.0], [200.0, 0.0]],
                10.0,
                "Samina",
            ),
        ];
        let chains = river_chains(&rs);
        assert_eq!(chains.len(), 2);
        assert_eq!(chains[0], vec![1, 0], "upstream first");
        assert_eq!(chains[1], vec![2]);
    }

    /// The DEM along a river has a 3 m bump (a bridge, trees): the level
    /// passes it without rising, and the tributary ends at the main river's
    /// level.
    #[test]
    fn rivers_flow_downhill_and_tributaries_meet_them() {
        let rs = vec![
            river(
                1,
                vec![1, 2],
                vec![[0.0, 0.0], [1000.0, 0.0]],
                20.0,
                "Rhein",
            ),
            river(
                2,
                vec![5, 6],
                vec![[500.0, -300.0], [500.0, -5.0]],
                10.0,
                "Samina",
            ),
        ];
        let ground = |p: [f64; 2]| {
            if p[1] < -1.0 {
                // The tributary's valley floor ends 2 m below the main river.
                Some(450.0 + (p[1] + 5.0) * -0.01 - 2.0)
            } else {
                let bump = if (400.0..450.0).contains(&p[0]) {
                    3.0
                } else {
                    0.0
                };
                Some(460.0 - p[0] * 0.01 + bump)
            }
        };
        let levels = river_levels(&rs, &ground);
        assert_eq!(levels.len(), 2);
        let main = &levels[0];
        assert!(main.levels.windows(2).all(|w| w[1] <= w[0] + 1e-9));
        let at = |x: f64| main.level_near([x, 0.0], 20.0).unwrap().1;
        assert!(at(425.0) < at(380.0) + 1e-9, "no step up at the bump");
        let trib = &levels[1];
        let confluence = at(500.0);
        assert!(trib.levels.iter().all(|&l| l >= confluence - 1e-9));
        assert!(trib.levels.windows(2).all(|w| w[1] <= w[0] + 1e-9));
    }

    fn water_pkg() -> WorldPackage {
        let n = 101usize;
        let key = TileKey { x: 0, z: 0 };
        let mut tiles = std::collections::BTreeMap::new();
        tiles.insert(
            key,
            Tile {
                key,
                heights: vec![400.0; n * n],
                roads: vec![],
            },
        );
        // A lake in the middle (vertices 40..60 × 40..60) with a tilted
        // level from the old mean-of-neighbours rule, carved 1 m below it.
        let mut water = Vec::new();
        for k in 40..=60 {
            for i in 40..=60 {
                let idx = (k * n + i) as u32;
                let level = 400.0 + (i as f32 - 50.0) * 0.1;
                water.push((idx, level));
                tiles.get_mut(&key).unwrap().heights[idx as usize] = level - 1.0;
            }
        }
        water.sort_by_key(|w| w.0);
        WorldPackage {
            name: "t".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: n as u32,
            tiles,
            graph: None,
            landuse: Some(LandUse {
                cells: 200,
                tiles: [(
                    key,
                    LandUseTile {
                        classes: vec![LandClass::Default; 200 * 200],
                        water,
                    },
                )]
                .into(),
            }),
            buildings: None,
            bridges: None,
            time_zone: None,
            routes: vec![],
            region: None,
            look: Default::default(),
        }
    }

    #[test]
    fn lakes_are_level_and_roads_are_not_carved() {
        let mut pkg = water_pkg();
        let ring = vec![
            [405.0, 405.0],
            [595.0, 405.0],
            [595.0, 595.0],
            [405.0, 595.0],
        ];
        let bodies = WaterBodies {
            rivers: vec![],
            lakes: vec![Lake {
                osm_id: 7,
                bbox: [405.0, 405.0, 595.0, 595.0],
                rings: vec![ring],
            }],
        };
        // A road along z = 500 (vertices on it keep the road height 400.2).
        let road = |p: [f64; 2]| ((p[1] - 500.0).abs() < 4.0).then_some(400.2);
        let s = level_water(&mut pkg, &bodies, &|_| Some(399.5), &road);
        assert_eq!(s.lakes, 1);
        assert!(s.restored_under_roads > 0);
        let lu = &pkg.landuse.as_ref().unwrap().tiles[&TileKey { x: 0, z: 0 }];
        let levels: Vec<f32> = lu.water.iter().map(|w| w.1).collect();
        assert!(
            levels.iter().all(|&l| (l - 399.5).abs() < 1e-4),
            "one level"
        );
        let n = 101;
        let t = &pkg.tiles[&TileKey { x: 0, z: 0 }];
        assert!((t.heights[45 * n + 45] - (399.5 - WATER_DEPTH_M as f32)).abs() < 1e-4);
        // Under the road: the road-conformed height, not a hole.
        assert!((t.heights[50 * n + 50] - 400.2).abs() < 1e-4);
        // The lint agrees: the lake is level now.
        let r = crate::lint::lint(&pkg, Some(&bodies));
        assert_eq!(r.count("water.lake_not_level"), 0);
        let before = crate::lint::lint(&water_pkg(), Some(&bodies));
        assert_eq!(before.count("water.lake_not_level"), 1);
    }

    /// Tenerife (spec §4.6, the sea): a river (barranco) running into the
    /// sea re-levels its own vertices but never the sea's, which stay at
    /// 0 m with the floor 2 m down.
    #[test]
    fn a_river_mouth_does_not_lift_the_sea() {
        let mut pkg = water_pkg();
        let n = 101;
        let key = TileKey { x: 0, z: 0 };
        // Sea in the rows z <= 100 m (k 0..=10), carved 2 m below 0 m.
        {
            let lu = pkg.landuse.as_mut().unwrap().tiles.get_mut(&key).unwrap();
            let t = pkg.tiles.get_mut(&key).unwrap();
            for k in 0..=10 {
                for i in 0..n {
                    let idx = (k * n + i) as u32;
                    lu.water.push((idx, 0.0));
                    t.heights[idx as usize] = -(crate::landuse::SEA_DEPTH_M as f32);
                }
            }
            lu.water.sort_by_key(|w| w.0);
        }
        // A river from z 600 m down to the coast at x = 500 m.
        let bodies = WaterBodies {
            rivers: vec![river(
                1,
                vec![1, 2],
                vec![[500.0, 600.0], [500.0, 50.0]],
                12.0,
                "Barranco",
            )],
            lakes: vec![],
        };
        level_water(&mut pkg, &bodies, &|p| Some(p[1] / 100.0 + 3.0), &|_| None);
        let lu = &pkg.landuse.as_ref().unwrap().tiles[&key];
        let t = &pkg.tiles[&key];
        for (i, k) in [(50, 5), (50, 0), (0, 10)] {
            let idx = (k * n + i) as u32;
            assert_eq!(lu.water_level(idx), Some(0.0), "{i} {k}");
            assert_eq!(
                t.heights[idx as usize],
                -(crate::landuse::SEA_DEPTH_M as f32)
            );
        }
    }

    #[test]
    fn rivers_and_lakes_are_told_apart() {
        let t = |pairs: &[(&str, &str)]| -> Tags {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        assert!(is_river_surface(&t(&[
            ("natural", "water"),
            ("water", "river")
        ])));
        assert!(is_river_surface(&t(&[("waterway", "riverbank")])));
        assert!(!is_river_surface(&t(&[
            ("natural", "water"),
            ("water", "lake")
        ])));
        assert!(!is_river_surface(&t(&[("natural", "water")])));
    }
}
