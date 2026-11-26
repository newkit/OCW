//! Shorelines from the OSM outlines (spec 4.6 *Water surfaces*, v6.58).
//!
//! The land-use step marks the terrain vertices in water and carves them a
//! fixed depth below the level; the client draws a flat quad over every
//! grid cell touching water, and the shore is where the terrain rises
//! through it. With one vertex in water and its neighbour on land the
//! crossing lies wherever the two heights put it – on a 10 m grid the
//! shoreline followed the grid's cells, a staircase (every lake and river
//! zig-zagged), and where the ground beside a lake lay below its level
//! (the DEM's noise, a lake level taken as the median) the water stood
//! over the land as a slab with stepped walls.
//!
//! **The fit:** the terrain near a shoreline follows its *signed
//! distance* to the outline. Every vertex within [`BAND_CELLS`] grid steps
//! of the outline (inside or out) gets the height `level + SLOPE · d`
//! (`d` < 0 in water, down to the water body's depth), so on every
//! triangle across the shore the drawn terrain crosses the water level
//! where the outline is: the linear interpolation of a distance field is
//! exact along a straight outline and within a few decimetres along a
//! curved one. The band's vertices all get a water level, so the cells
//! along the outline are drawn even where no vertex lies in the water (a
//! river 8 m wide between the 10 m vertices).
//!
//! - **Outlines:** the OSM water areas (lakes, reservoirs, ponds,
//!   riverbanks, wide rivers drawn as areas; even-odd rings) and the sea
//!   ([`crate::landuse::sea`], level 0 m). Where outlines overlap the
//!   union counts (the minimum of the signed distances).
//! - **Rivers and streams** drawn as lines are not fitted on the grid: a
//!   river 12 m wide has no room for a distance field between 10 m
//!   vertices (both of its shores cross one cell). They become ribbons
//!   along their course ([`rivers`], section `WATR`): the wide ones at
//!   their level over a carved channel, the streams draped on the ground.
//! - **Levels:** each vertex keeps the level it has; a band vertex
//!   without one takes the nearest level within three grid steps (inside
//!   the water first); the sea keeps 0 m.
//! - **Kept:** vertices under a road (its half width plus a grid step),
//!   under junction surfaces and around building footprints keep their
//!   height (they still get a level, so the water reaches them).

use std::collections::{BTreeMap, HashMap, HashSet};

use ocw_world::package::{TileKey, WorldPackage};

use crate::landuse::{Area, SEA_DEPTH_M, SURFACE_MIN_WIDTH_M, WATER_DEPTH_M, WaterLine};

/// Rise of the terrain per metre across a shoreline (m/m): the flattest
/// bank of the fit (0.1 × 15 m = 1.5 m at the band's edge) …
pub const SLOPE: f64 = 0.1;
/// … and the steepest. Each water body's fitted banks take one slope, the
/// median rise over distance of its land vertices in the band within these
/// bounds: one slope for all its band keeps the drawn shore on the outline
/// (the distance field stays linear), the median keeps a lake in steep
/// country from being ringed by a cut-down terrace.
pub const MAX_SLOPE: f64 = 0.5;
/// Width of the fitted band on either side of the outline, in grid steps:
/// every vertex of a cell the outline crosses lies within √2 steps.
pub const BAND_CELLS: f64 = 1.5;
/// The fit moves a vertex by at most this much (m): a steep bank or a dam
/// beside the water keeps its shape (its shore follows the grid there,
/// where the slope hides it), instead of a pit or a wall.
pub const MAX_CHANGE_M: f32 = 3.0;
/// How far (grid steps) a band vertex without a level looks for one.
const LEVEL_SEARCH: i64 = 3;

/// One water outline.
#[derive(Debug, Clone, PartialEq)]
pub enum Outline {
    /// Closed rings (even-odd), the sea if `sea`.
    Area {
        rings: Vec<Vec<[f64; 2]>>,
        sea: bool,
    },
    /// A centre line with its half width.
    Line { points: Vec<[f64; 2]>, half: f64 },
}

impl Outline {
    fn segments(&self) -> Vec<([f64; 2], [f64; 2])> {
        match self {
            Outline::Area { rings, .. } => rings
                .iter()
                .flat_map(|r| (0..r.len()).map(move |j| (r[j], r[(j + 1) % r.len()])))
                .collect(),
            Outline::Line { points, .. } => points.windows(2).map(|w| (w[0], w[1])).collect(),
        }
    }

    /// The mean width of an area (twice its area over its perimeter; a
    /// line's width): rivers drawn as areas are narrow, lakes wide.
    pub fn mean_width(&self) -> f64 {
        match self {
            Outline::Line { half, .. } => 2.0 * half,
            Outline::Area { rings, .. } => {
                let ring_area = |r: &Vec<[f64; 2]>| {
                    (0..r.len())
                        .map(|j| {
                            let (a, b) = (r[j], r[(j + 1) % r.len()]);
                            a[0] * b[1] - b[0] * a[1]
                        })
                        .sum::<f64>()
                        .abs()
                        / 2.0
                };
                let area = rings.first().map_or(0.0, ring_area)
                    - rings.iter().skip(1).map(ring_area).sum::<f64>();
                let perimeter: f64 = self
                    .segments()
                    .iter()
                    .map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1]))
                    .sum();
                if perimeter > 0.0 {
                    2.0 * area.max(0.0) / perimeter
                } else {
                    0.0
                }
            }
        }
    }

    fn is_sea(&self) -> bool {
        matches!(self, Outline::Area { sea: true, .. })
    }

    fn depth(&self) -> f64 {
        if self.is_sea() {
            SEA_DEPTH_M
        } else {
            WATER_DEPTH_M
        }
    }

    /// Signed distance of `p` (negative in the water).
    pub fn signed_distance(&self, p: [f64; 2]) -> f64 {
        match self {
            Outline::Area { rings, .. } => {
                let d = self
                    .segments()
                    .iter()
                    .map(|&(a, b)| crate::spatial::foot_on_segment(p, a, b).0)
                    .fold(f64::INFINITY, f64::min);
                if crate::areas::inside_rings(p, rings) {
                    -d
                } else {
                    d
                }
            }
            Outline::Line { points, half } => {
                let d = points
                    .windows(2)
                    .map(|w| crate::spatial::foot_on_segment(p, w[0], w[1]).0)
                    .fold(f64::INFINITY, f64::min);
                d - half
            }
        }
    }
}

/// The outlines of the water surfaces: the water areas of `areas` and the
/// sea (the rivers drawn as lines are ribbons, [`rivers`]).
pub fn outlines(areas: &[Area], sea: Option<&Area>) -> Vec<Outline> {
    use ocw_world::landuse::LandClass;
    let mut out: Vec<Outline> = areas
        .iter()
        .filter(|a| a.class == LandClass::Water && a.level.is_none())
        .map(|a| Outline::Area {
            rings: a.rings.clone(),
            sea: false,
        })
        .collect();
    if let Some(s) = sea {
        out.push(Outline::Area {
            rings: s.rings.clone(),
            sea: true,
        });
    }
    out
}

/// What [`fit_shores`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShoreStats {
    pub outlines: usize,
    /// Vertices within the band of an outline (each grid vertex once).
    pub band: usize,
    /// Band vertices given the shore's height, raised and lowered.
    pub raised: usize,
    pub lowered: usize,
    /// Band vertices kept (roads, junctions, buildings).
    pub kept: usize,
    /// Band vertices without a level near them (left as they were).
    pub no_level: usize,
    /// Vertices that got a water level they did not have.
    pub new_levels: usize,
    /// Land vertices of the band left below the water by
    /// [`MAX_CHANGE_M`]: no level (no water drawn over them).
    pub dry_kept: usize,
    pub max_raise_m: f64,
    pub max_lower_m: f64,
}

/// A global grid vertex.
type Vertex = (i64, i64);

/// A change of a vertex in a tile: its index, the new height (none: kept)
/// and its water level (none: dropped).
type VertexChange = (usize, Option<f32>, Option<f32>);

/// A band vertex's plan: the vertex, kept, its height, the fitted height,
/// its water level and its signed distance.
type Planned = (Vertex, bool, f32, f32, f32, f64);

/// The tiles holding global vertex `v` and its index in each.
fn holders(pkg: &WorldPackage, v: Vertex) -> Vec<(TileKey, usize)> {
    let n = pkg.samples as usize;
    let step = (n - 1) as i64;
    let mut out = Vec::new();
    let (tx, tz) = (v.0.div_euclid(step), v.1.div_euclid(step));
    let (i, k) = (v.0.rem_euclid(step), v.1.rem_euclid(step));
    let xs: Vec<(i64, i64)> = if i == 0 {
        vec![(tx, 0), (tx - 1, step)]
    } else {
        vec![(tx, i)]
    };
    let zs: Vec<(i64, i64)> = if k == 0 {
        vec![(tz, 0), (tz - 1, step)]
    } else {
        vec![(tz, k)]
    };
    for &(x, ii) in &xs {
        for &(z, kk) in &zs {
            let key = TileKey {
                x: x as i32,
                z: z as i32,
            };
            if pkg
                .tiles
                .get(&key)
                .is_some_and(|t| t.heights.len() == n * n)
            {
                out.push((key, kk as usize * n + ii as usize));
            }
        }
    }
    out
}

/// The stored water level and the height of global vertex `v`.
fn vertex_state(pkg: &WorldPackage, v: Vertex) -> Option<(Option<f32>, f32)> {
    let hs = holders(pkg, v);
    let &(key, idx) = hs.first()?;
    let h = pkg.tiles[&key].heights[idx];
    let level = hs.iter().find_map(|&(key, idx)| {
        pkg.landuse
            .as_ref()?
            .tiles
            .get(&key)?
            .water_level(idx as u32)
    });
    Some((level, h))
}

/// Signed distance of the band vertices to the union of `outlines`, with
/// the outline that decides it. Vertices farther than `band` on either
/// side are not listed.
pub fn band_distances(
    pkg: &WorldPackage,
    outlines: &[Outline],
    band: f64,
) -> HashMap<Vertex, (f64, usize)> {
    let sp = pkg.spacing_m();
    let exists = |v: Vertex| !holders(pkg, v).is_empty();
    let per_outline = crate::par::map(outlines, crate::par::threads(), |o| {
        // Unsigned distance within the band, per vertex.
        let mut dist: HashMap<Vertex, f64> = HashMap::new();
        for (a, b) in o.segments() {
            let extra = match o {
                Outline::Line { half, .. } => *half,
                Outline::Area { .. } => 0.0,
            };
            let r = band + extra;
            let (i0, i1) = (
                ((a[0].min(b[0]) - r) / sp).ceil() as i64,
                ((a[0].max(b[0]) + r) / sp).floor() as i64,
            );
            let (k0, k1) = (
                ((a[1].min(b[1]) - r) / sp).ceil() as i64,
                ((a[1].max(b[1]) + r) / sp).floor() as i64,
            );
            // Long straight edges (the sea's frame) would touch millions
            // of vertices: only the package's are measured.
            if (i1 - i0) * (k1 - k0) > 4_000_000 {
                let mut cells: Vec<Vertex> = Vec::new();
                for key in pkg.tiles.keys() {
                    let step = i64::from(pkg.samples) - 1;
                    let (gx, gz) = (i64::from(key.x) * step, i64::from(key.z) * step);
                    if gx + step < i0 || gx > i1 || gz + step < k0 || gz > k1 {
                        continue;
                    }
                    for k in gz.max(k0)..=(gz + step).min(k1) {
                        for i in gx.max(i0)..=(gx + step).min(i1) {
                            cells.push((i, k));
                        }
                    }
                }
                for v in cells {
                    let p = [v.0 as f64 * sp, v.1 as f64 * sp];
                    let d = crate::spatial::foot_on_segment(p, a, b).0;
                    if d <= r {
                        let e = dist.entry(v).or_insert(f64::INFINITY);
                        *e = e.min(d);
                    }
                }
                continue;
            }
            for k in k0..=k1 {
                for i in i0..=i1 {
                    let p = [i as f64 * sp, k as f64 * sp];
                    let d = crate::spatial::foot_on_segment(p, a, b).0;
                    if d <= r && exists((i, k)) {
                        let e = dist.entry((i, k)).or_insert(f64::INFINITY);
                        *e = e.min(d);
                    }
                }
            }
        }
        // Signs: inside the rings (scanlines over the rows that have band
        // vertices, the rule of `landuse::fill_area`), or the line's width.
        let mut signed: Vec<(Vertex, f64)> = Vec::with_capacity(dist.len());
        match o {
            Outline::Line { half, .. } => {
                for (v, d) in dist {
                    let s = d - half;
                    if s.abs() <= band {
                        signed.push((v, s));
                    }
                }
            }
            Outline::Area { rings, .. } => {
                let mut rows: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
                for v in dist.keys() {
                    rows.entry(v.1).or_default();
                }
                for ring in rings {
                    let m = ring.len();
                    for j in 0..m {
                        let (a, b) = (ring[j], ring[(j + 1) % m]);
                        if a[1] == b[1] {
                            continue;
                        }
                        let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
                        let k0 = (lo / sp).ceil() as i64;
                        let k1 = (hi / sp).ceil() as i64; // exclusive
                        for (&k, xs) in rows.range_mut(k0..k1) {
                            let z = k as f64 * sp;
                            let t = (z - a[1]) / (b[1] - a[1]);
                            xs.push(a[0] + t * (b[0] - a[0]));
                        }
                    }
                }
                for (v, d) in dist {
                    let x = v.0 as f64 * sp;
                    let crossings = rows[&v.1].iter().filter(|&&c| c <= x).count();
                    let s = if crossings % 2 == 1 { -d } else { d };
                    if s.abs() <= band {
                        signed.push((v, s));
                    }
                }
            }
        }
        signed
    });
    let mut out: HashMap<Vertex, (f64, usize)> = HashMap::new();
    for (oi, list) in per_outline.into_iter().enumerate() {
        for (v, d) in list {
            match out.get_mut(&v) {
                Some(e) if e.0 <= d => {}
                Some(e) => *e = (d, oi),
                None => {
                    out.insert(v, (d, oi));
                }
            }
        }
    }
    out
}

/// The vertices whose height the fit keeps: under the junction surfaces
/// (`surfaces`, the corners of their cells) and around the building
/// footprints (their box grown by a metre and a grid step), as global
/// vertices; the roads are the caller's `on_road`.
pub fn kept_vertices(pkg: &WorldPackage, surfaces: &[[f64; 3]]) -> HashSet<Vertex> {
    let sp = pkg.spacing_m();
    let mut out = HashSet::new();
    for q in surfaces {
        let (gi, gk) = ((q[0] / sp).floor() as i64, (q[2] / sp).floor() as i64);
        for (di, dk) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            out.insert((gi + di, gk + dk));
        }
    }
    if let Some(b) = &pkg.buildings {
        let outlines: Vec<Vec<[f64; 2]>> = b
            .values()
            .flatten()
            .map(|bd| bd.outline.iter().map(|p| p.map(f64::from)).collect())
            .collect();
        out.extend(kept_around_footprints(&outlines, sp));
    }
    out
}

/// The vertices around building footprints (each one's box grown by a
/// metre and a grid step `sp`).
pub fn kept_around_footprints(footprints: &[Vec<[f64; 2]>], sp: f64) -> HashSet<Vertex> {
    let mut out = HashSet::new();
    for outline in footprints {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in outline {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        if !lo[0].is_finite() {
            continue;
        }
        let g = 1.0 + sp;
        let (a0, a1) = (
            ((lo[0] - g) / sp).floor() as i64,
            ((lo[1] - g) / sp).floor() as i64,
        );
        let (b0, b1) = (
            ((hi[0] + g) / sp).ceil() as i64,
            ((hi[1] + g) / sp).ceil() as i64,
        );
        for gk in a1..=b1 {
            for gi in a0..=b0 {
                out.insert((gi, gk));
            }
        }
    }
    out
}

/// Fits the terrain along the shorelines of `outlines` (see the module).
/// `kept(v)` / `on_road(p)`: vertices whose height stays.
pub fn fit_shores(
    pkg: &mut WorldPackage,
    outlines: &[Outline],
    kept: &HashSet<Vertex>,
    on_road: &(dyn Fn([f64; 2]) -> bool + Sync),
) -> ShoreStats {
    let mut st = ShoreStats {
        outlines: outlines.len(),
        ..Default::default()
    };
    if pkg.landuse.is_none() || pkg.tiles.is_empty() {
        return st;
    }
    let sp = pkg.spacing_m();
    let band = BAND_CELLS * sp;
    let dists = band_distances(pkg, outlines, band);
    let mut verts: Vec<(Vertex, f64, usize)> =
        dists.iter().map(|(v, (d, o))| (*v, *d, *o)).collect();
    verts.sort_by_key(|a| a.0);
    // The new (height, level) of each band vertex.
    let shared: &WorldPackage = pkg;
    let changes = crate::par::map(&verts, crate::par::threads(), |&(v, d, oi)| {
        let pkg = shared;
        let o = &outlines[oi];
        let (level, h) = vertex_state(pkg, v)?;
        let level = match (level, o.is_sea()) {
            (_, true) => crate::landuse::SEA_LEVEL_M as f32,
            (Some(l), false) => l,
            (None, false) => {
                // The nearest level, a vertex in the water first.
                let mut best: Option<(bool, i64, f32)> = None;
                for dk in -LEVEL_SEARCH..=LEVEL_SEARCH {
                    for di in -LEVEL_SEARCH..=LEVEL_SEARCH {
                        let w = (v.0 + di, v.1 + dk);
                        let Some((Some(l), _)) = vertex_state(pkg, w) else {
                            continue;
                        };
                        let wet = dists.get(&w).is_some_and(|e| e.0 < 0.0);
                        let key = (!wet, di * di + dk * dk, l);
                        if best.is_none_or(|b| (key.0, key.1) < (b.0, b.1)) {
                            best = Some(key);
                        }
                    }
                }
                best?.2
            }
        };
        let p = [v.0 as f64 * sp, v.1 as f64 * sp];
        let keep = kept.contains(&v) || on_road(p);
        Some((v, keep, h, level, d, oi))
    });
    // Each water body's bank slope: the median rise over distance of its
    // land vertices in the band.
    let mut ratios: HashMap<usize, Vec<f64>> = HashMap::new();
    for &(_, keep, h, level, d, oi) in changes.iter().flatten() {
        if d >= 2.0 && !keep {
            ratios.entry(oi).or_default().push(f64::from(h - level) / d);
        }
    }
    let slope: HashMap<usize, f64> = ratios
        .into_iter()
        .map(|(oi, mut r)| {
            r.sort_by(f64::total_cmp);
            (oi, r[r.len() / 2].clamp(SLOPE, MAX_SLOPE))
        })
        .collect();
    let changes: Vec<Option<Planned>> = changes
        .into_iter()
        .map(|c| {
            let (v, keep, h, level, d, oi) = c?;
            let k = slope.get(&oi).copied().unwrap_or(SLOPE);
            // Under the water the bank goes on down to the depth of the
            // water body, deeper along a steep shore (the band needs it).
            let floor = -outlines[oi].depth().max(k * BAND_CELLS * sp);
            let target = f64::from(level) + (k * d).max(floor);
            Some((v, keep, h, target as f32, level, d))
        })
        .collect();
    let mut by_tile: HashMap<TileKey, Vec<VertexChange>> = HashMap::new();
    for (c, &(v, _, _)) in changes.iter().zip(&verts) {
        st.band += 1;
        let Some((v2, keep, h, target, level, d)) = *c else {
            st.no_level += 1;
            continue;
        };
        debug_assert_eq!(v, v2);
        let capped = target.clamp(h - MAX_CHANGE_M, h + MAX_CHANGE_M);
        // Land that stays under the water (a dam's foot beside a lake)
        // gets no level: no water is drawn over it.
        let wet_land = d > 0.0 && !keep && capped < target - 1e-3 && capped <= level + 0.05;
        let target = capped;
        if wet_land {
            st.dry_kept += 1;
            for (key, idx) in holders(pkg, v) {
                by_tile.entry(key).or_default().push((idx, None, None));
            }
            continue;
        }
        let new_h = if keep {
            st.kept += 1;
            None
        } else {
            if target > h {
                st.raised += 1;
                st.max_raise_m = st.max_raise_m.max(f64::from(target - h));
            } else if target < h {
                st.lowered += 1;
                st.max_lower_m = st.max_lower_m.max(f64::from(h - target));
            }
            Some(target)
        };
        if vertex_state(pkg, v).is_some_and(|s| s.0.is_none()) {
            st.new_levels += 1;
        }
        for (key, idx) in holders(pkg, v) {
            by_tile
                .entry(key)
                .or_default()
                .push((idx, new_h, Some(level)));
        }
    }
    let lu = pkg.landuse.as_mut().expect("land use");
    for (key, list) in by_tile {
        if let Some(t) = pkg.tiles.get_mut(&key) {
            for &(idx, h, _) in &list {
                if let Some(h) = h {
                    t.heights[idx] = h;
                }
            }
        }
        let Some(lt) = lu.tiles.get_mut(&key) else {
            continue;
        };
        let mut levels: BTreeMap<u32, f32> = lt.water.iter().copied().collect();
        for &(idx, _, level) in &list {
            match level {
                Some(l) => levels.insert(idx as u32, l),
                None => levels.remove(&(idx as u32)),
            };
        }
        lt.water = levels.into_iter().collect();
    }
    st
}

/// Rivers, streams and canals at least this wide are drawn as ribbons
/// along their course (section `WATR`).
pub const RIBBON_MIN_WIDTH_M: f64 = 2.0;
/// Spacing of the ribbons' points (m).
pub const RIBBON_STEP_M: f64 = 5.0;
/// A ribbon narrower than [`SURFACE_MIN_WIDTH_M`] (a stream) lies on the
/// ground (the client drapes it over the drawn terrain): its stored level
/// is this far below the ground, so the ground decides.
pub const DRAPED_BELOW_M: f64 = 5.0;
/// The water of a wide river's ribbon is this deep at its channel.
pub const CHANNEL_DEPTH_M: f64 = WATER_DEPTH_M;

/// What [`rivers`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RiverStats {
    /// Lines drawn (wide ones with a level, the rest draped) and pieces.
    pub wide: usize,
    pub draped: usize,
    pub pieces: usize,
    pub km: f64,
    /// Points left out inside lakes, riverbanks and the sea.
    pub in_areas: usize,
    /// Vertices carved under a wide river's ribbon, raised beside it.
    pub carved: usize,
    pub raised: usize,
    /// Land-use water levels of the wide rivers dropped (the ribbons
    /// replace the grid's water there).
    pub levels_dropped: usize,
}

/// Bounding box of an outline (min x, min z, max x, max z).
fn outline_box(o: &Outline) -> [f64; 4] {
    let pts: Vec<[f64; 2]> = match o {
        Outline::Area { rings, .. } => rings.iter().flatten().copied().collect(),
        Outline::Line { points, half } => points
            .iter()
            .flat_map(|p| [[p[0] - half, p[1] - half], [p[0] + half, p[1] + half]])
            .collect(),
    };
    pts.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

/// Whether `p` lies in a lake, a riverbank area or the sea of `areas`
/// (the sea by the land use: its vertices' level is 0 m).
struct InWater<'a> {
    areas: &'a [Outline],
    buckets: HashMap<(i64, i64), Vec<usize>>,
}

const IN_WATER_BUCKET_M: f64 = 500.0;

/// Water areas narrower than this many grid steps on average
/// ([`Outline::mean_width`]) are rivers drawn as areas: a river's ribbon
/// runs through them at their width (their distance field has no room on
/// the grid, both shores cross one cell).
pub const NARROW_CELLS: f64 = 3.0;
/// The widest a ribbon gets in a narrow area (m).
pub const RIBBON_MAX_WIDTH_M: f64 = 60.0;

impl<'a> InWater<'a> {
    fn new(areas: &'a [Outline]) -> Self {
        let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, o) in areas.iter().enumerate() {
            if o.is_sea() {
                continue;
            }
            let b = outline_box(o);
            let c = |v: f64| (v / IN_WATER_BUCKET_M).floor() as i64;
            for x in c(b[0])..=c(b[2]) {
                for z in c(b[1])..=c(b[3]) {
                    buckets.entry((x, z)).or_default().push(i);
                }
            }
        }
        Self { areas, buckets }
    }

    /// Half the width of the area around `p` if `p` lies in one: its
    /// distance to the area's outline.
    fn half_width_at(&self, p: [f64; 2]) -> Option<f64> {
        let key = (
            (p[0] / IN_WATER_BUCKET_M).floor() as i64,
            (p[1] / IN_WATER_BUCKET_M).floor() as i64,
        );
        self.buckets.get(&key)?.iter().find_map(|&i| {
            let o = &self.areas[i];
            let d = o.signed_distance(p);
            (d < 0.0).then_some(-d)
        })
    }

    fn contains(&self, pkg: &WorldPackage, p: [f64; 2]) -> bool {
        let sp = pkg.spacing_m();
        let v = ((p[0] / sp).round() as i64, (p[1] / sp).round() as i64);
        if vertex_state(pkg, v).is_some_and(|s| s.0 == Some(crate::landuse::SEA_LEVEL_M as f32)) {
            return true;
        }
        let key = (
            (p[0] / IN_WATER_BUCKET_M).floor() as i64,
            (p[1] / IN_WATER_BUCKET_M).floor() as i64,
        );
        self.buckets.get(&key).is_some_and(|list| {
            list.iter().any(|&i| match &self.areas[i] {
                Outline::Area { rings, .. } => crate::areas::inside_rings(p, rings),
                Outline::Line { .. } => false,
            })
        })
    }
}

/// The rivers, streams and canals of `lines` (at least
/// [`RIBBON_MIN_WIDTH_M`] wide) as ribbons along their course into the
/// section `WATR` (see [`ocw_world::water_lines`]): the wide ones (at least
/// [`SURFACE_MIN_WIDTH_M`]) at their level – the land use's water levels
/// along them, else the drawn ground, never rising downstream – with the
/// terrain carved under them and raised beside them, and their grid water
/// levels dropped (the ribbon replaces the grid's staircase); the narrower
/// ones draped on the ground. Left out inside the lakes, riverbank areas
/// and the sea of `areas` (their surfaces are drawn there). Vertices in
/// `kept` or `on_road` keep their heights.
pub fn rivers(
    pkg: &mut WorldPackage,
    lines: &[WaterLine],
    areas: &[Outline],
    kept: &HashSet<Vertex>,
    on_road: &(dyn Fn([f64; 2]) -> bool + Sync),
) -> RiverStats {
    let mut st = RiverStats::default();
    let sp = pkg.spacing_m();
    // Lakes and the sea end a ribbon; narrow areas (rivers drawn as areas)
    // carry it at their width, their grid water replaced by it.
    let (wide_areas, narrow_areas): (Vec<Outline>, Vec<Outline>) = areas
        .iter()
        .cloned()
        .partition(|o| o.is_sea() || o.mean_width() >= NARROW_CELLS * sp);
    let in_water = InWater::new(&wide_areas);
    let in_narrow = InWater::new(&narrow_areas);
    let area_band = band_distances(pkg, &wide_areas, BAND_CELLS * sp);
    let level_at = |pkg: &WorldPackage, p: [f64; 2]| -> Option<f64> {
        let (fi, fk) = (p[0] / sp, p[1] / sp);
        let (i, k) = (fi.floor() as i64, fk.floor() as i64);
        let (tx, tz) = (fi - i as f64, fk - k as f64);
        let l = |di: i64, dk: i64| vertex_state(pkg, (i + di, k + dk)).and_then(|s| s.0);
        let (a, b, c, d) = (l(0, 0)?, l(1, 0)?, l(0, 1)?, l(1, 1)?);
        let (a, b, c, d) = (f64::from(a), f64::from(b), f64::from(c), f64::from(d));
        Some(if tx + tz <= 1.0 {
            a + tx * (b - a) + tz * (c - a)
        } else {
            d + (1.0 - tx) * (c - d) + (1.0 - tz) * (b - d)
        })
    };
    let mut ribbons: Vec<ocw_world::water_lines::WaterLine> = Vec::new();
    // Vertex → (channel height, bank height) wanted by the wide rivers.
    let mut channel: HashMap<Vertex, f64> = HashMap::new();
    let mut bank: HashMap<Vertex, f64> = HashMap::new();
    let mut drop: HashSet<Vertex> = HashSet::new();
    for l in lines
        .iter()
        .filter(|l| l.width_m >= RIBBON_MIN_WIDTH_M && l.points.len() >= 2)
    {
        let pts = crate::profile::resample(&l.points, RIBBON_STEP_M);
        if pts.len() < 2 {
            continue;
        }
        let wide = l.width_m >= SURFACE_MIN_WIDTH_M;
        let ground: Vec<Option<f64>> = pts
            .iter()
            .map(|&p| {
                if wide {
                    level_at(pkg, p).or_else(|| pkg.surface_height(p[0], p[1]))
                } else {
                    pkg.surface_height(p[0], p[1]).map(|h| h - DRAPED_BELOW_M)
                }
            })
            .collect();
        let Some(first) = ground.iter().flatten().next().copied() else {
            continue;
        };
        let raw: Vec<f64> = ground.iter().map(|g| g.unwrap_or(first)).collect();
        let levels = if wide {
            crate::water::non_increasing(&raw)
        } else {
            raw
        };
        let inside: Vec<bool> = pts.iter().map(|&p| in_water.contains(pkg, p)).collect();
        // The surface's width at each point: the line's, or the narrow
        // area's it runs through.
        let widths: Vec<f64> = pts
            .iter()
            .map(|&p| {
                in_narrow.half_width_at(p).map_or(l.width_m, |h| {
                    (2.0 * h).min(RIBBON_MAX_WIDTH_M).max(l.width_m)
                })
            })
            .collect();
        st.in_areas += inside.iter().filter(|x| **x).count();
        if wide {
            st.wide += 1;
        } else {
            st.draped += 1;
        }
        // Runs of points outside the areas, each reaching one point into
        // the water it meets (the ribbon ends at the shore).
        let mut j = 0;
        while j < pts.len() {
            if inside[j] {
                j += 1;
                continue;
            }
            let start = j.saturating_sub(1);
            while j < pts.len() && !inside[j] {
                j += 1;
            }
            let end = j.min(pts.len() - 1);
            if end <= start {
                continue;
            }
            let piece: Vec<[f32; 4]> = (start..=end)
                .map(|m| {
                    [
                        pts[m][0] as f32,
                        levels[m] as f32,
                        pts[m][1] as f32,
                        widths[m] as f32,
                    ]
                })
                .collect();
            st.km += pts[start..=end]
                .windows(2)
                .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
                .sum::<f64>()
                / 1000.0;
            if wide {
                for m in start..end {
                    let (a, b) = (pts[m], pts[m + 1]);
                    let (la, lb) = (levels[m], levels[m + 1]);
                    let half = widths[m].max(widths[m + 1]) / 2.0;
                    let r = half + BAND_CELLS * sp;
                    let (i0, i1) = (
                        ((a[0].min(b[0]) - r) / sp).ceil() as i64,
                        ((a[0].max(b[0]) + r) / sp).floor() as i64,
                    );
                    let (k0, k1) = (
                        ((a[1].min(b[1]) - r) / sp).ceil() as i64,
                        ((a[1].max(b[1]) + r) / sp).floor() as i64,
                    );
                    for k in k0..=k1 {
                        for i in i0..=i1 {
                            let p = [i as f64 * sp, k as f64 * sp];
                            let (d, t, _) = crate::spatial::foot_on_segment(p, a, b);
                            if d > r {
                                continue;
                            }
                            let lv = la + (lb - la) * t;
                            let v = (i, k);
                            // The grid's water of the river goes (not a
                            // lake's or the sea's beside it).
                            if !area_band.contains_key(&v) && !in_water.contains(pkg, p) {
                                drop.insert(v);
                            }
                            if d <= half + 0.5 * sp {
                                let e = channel.entry(v).or_insert(f64::INFINITY);
                                *e = e.min(lv - CHANNEL_DEPTH_M);
                            } else {
                                let e = bank.entry(v).or_insert(f64::NEG_INFINITY);
                                *e = e.max(lv + 0.2 + SLOPE * (d - half));
                            }
                        }
                    }
                }
            }
            ribbons.push(ocw_world::water_lines::WaterLine {
                width_m: l.width_m as f32,
                points: piece,
            });
        }
    }
    st.pieces = ribbons.len();
    // Heights: the channel first, the banks only raise.
    let mut by_tile: HashMap<TileKey, Vec<(usize, Option<f32>, bool)>> = HashMap::new();
    let mut verts: HashSet<Vertex> = channel.keys().copied().collect();
    verts.extend(bank.keys().copied());
    verts.extend(drop.iter().copied());
    for v in verts {
        let p = [v.0 as f64 * sp, v.1 as f64 * sp];
        let Some((_, h)) = vertex_state(pkg, v) else {
            continue;
        };
        let keep = kept.contains(&v) || on_road(p);
        // A lake's shore band is the lake's: carved for a river mouth, never raised.
        let lake_shore = area_band.contains_key(&v);
        let h = f64::from(h);
        let new_h = if keep {
            None
        } else if let Some(&c) = channel.get(&v) {
            (c < h).then(|| {
                st.carved += 1;
                c as f32
            })
        } else if let Some(&b) = bank.get(&v).filter(|_| !lake_shore) {
            (b > h).then(|| {
                st.raised += 1;
                b as f32
            })
        } else {
            None
        };
        let dropped = drop.contains(&v);
        if new_h.is_some() || dropped {
            for (key, idx) in holders(pkg, v) {
                by_tile.entry(key).or_default().push((idx, new_h, dropped));
            }
        }
    }
    for (key, list) in by_tile {
        if let Some(t) = pkg.tiles.get_mut(&key) {
            for &(idx, h, _) in &list {
                if let Some(h) = h {
                    t.heights[idx] = h;
                }
            }
        }
        if let Some(lt) = pkg.landuse.as_mut().and_then(|lu| lu.tiles.get_mut(&key)) {
            let gone: HashSet<u32> = list.iter().filter(|e| e.2).map(|e| e.0 as u32).collect();
            let before = lt.water.len();
            lt.water.retain(|w| !gone.contains(&w.0));
            st.levels_dropped += before - lt.water.len();
        }
    }
    let lines =
        ocw_world::water_lines::WaterLines::from_lines(&ribbons, f64::from(pkg.tile_size_m));
    pkg.look.water_lines = (!lines.tiles.is_empty()).then_some(lines);
    st
}

/// How far the drawn shoreline lies from the outline: at points every
/// `step` metres along each outline (the package's tiles, not under
/// `skip`, not inside other water), the distance along the outline's
/// normal to the nearest place where the drawn terrain
/// ([`WorldPackage::surface_height`]) crosses the water level (the stored
/// levels interpolated over the cell like the drawn water), searched
/// within `reach` metres (`reach` where there is none). Sorted ascending.
pub fn shore_offsets(
    pkg: &WorldPackage,
    outlines: &[Outline],
    step: f64,
    reach: f64,
    skip: &dyn Fn([f64; 2]) -> bool,
) -> Vec<f64> {
    shore_offsets_at(pkg, outlines, step, reach, skip)
        .into_iter()
        .map(|o| o.0)
        .collect()
}

/// [`shore_offsets`] with the point and the outline (its index) of each
/// (sorted by the offset).
pub fn shore_offsets_at(
    pkg: &WorldPackage,
    outlines: &[Outline],
    step: f64,
    reach: f64,
    skip: &dyn Fn([f64; 2]) -> bool,
) -> Vec<(f64, [f64; 2], usize)> {
    let sp = pkg.spacing_m();
    let dists = band_distances(pkg, outlines, BAND_CELLS * sp);
    let level_at = |p: [f64; 2]| -> Option<f64> {
        let (fi, fk) = (p[0] / sp, p[1] / sp);
        let (i, k) = (fi.floor() as i64, fk.floor() as i64);
        let (tx, tz) = (fi - i as f64, fk - k as f64);
        let l = |di: i64, dk: i64| vertex_state(pkg, (i + di, k + dk)).and_then(|s| s.0);
        let (a, b, c, d) = (l(0, 0)?, l(1, 0)?, l(0, 1)?, l(1, 1)?);
        let (a, b, c, d) = (f64::from(a), f64::from(b), f64::from(c), f64::from(d));
        Some(if tx + tz <= 1.0 {
            a + tx * (b - a) + tz * (c - a)
        } else {
            d + (1.0 - tx) * (c - d) + (1.0 - tz) * (b - d)
        })
    };
    // A point on the union's shore: the corners of its cell are all in the
    // band, not all in the water.
    let on_shore = |p: [f64; 2]| {
        let (i, k) = ((p[0] / sp).floor() as i64, (p[1] / sp).floor() as i64);
        let ds: Vec<f64> = [(0, 0), (1, 0), (0, 1), (1, 1)]
            .iter()
            .filter_map(|&(di, dk)| dists.get(&(i + di, k + dk)).map(|e| e.0))
            .collect();
        ds.len() == 4 && ds.iter().any(|d| *d >= 0.0) && ds.iter().any(|d| *d < 0.0)
    };
    let wet = |p: [f64; 2]| -> Option<f64> { Some(pkg.surface_height(p[0], p[1])? - level_at(p)?) };
    let mut out = Vec::new();
    for (oi, o) in outlines.iter().enumerate() {
        let half = match o {
            Outline::Line { half, .. } => *half,
            Outline::Area { .. } => 0.0,
        };
        for (a, b) in o.segments() {
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            if len < 1e-9 {
                continue;
            }
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let normal = [-dir[1], dir[0]];
            let count = (len / step).ceil().max(1.0) as usize;
            for s in 0..count {
                let t = (s as f64 + 0.5) / count as f64 * len;
                let centre = [a[0] + dir[0] * t, a[1] + dir[1] * t];
                let sides: &[f64] = if half > 0.0 { &[-1.0, 1.0] } else { &[1.0] };
                for &side in sides {
                    let q = [
                        centre[0] + normal[0] * half * side,
                        centre[1] + normal[1] * half * side,
                    ];
                    if skip(q) || !on_shore(q) || wet(q).is_none() {
                        continue;
                    }
                    let mut best = reach;
                    let mut prev: Option<(f64, f64)> = None;
                    let mut x = -reach;
                    while x <= reach + 1e-9 {
                        let r = [q[0] + normal[0] * x, q[1] + normal[1] * x];
                        if let Some(e) = wet(r) {
                            if let Some((px, pe)) = prev
                                && (pe <= 0.0) != (e <= 0.0)
                            {
                                let z = px + (x - px) * pe / (pe - e);
                                best = best.min(z.abs());
                            }
                            prev = Some((x, e));
                        } else {
                            prev = None;
                        }
                        x += 0.25;
                    }
                    out.push((best, q, oi));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

#[cfg(test)]
mod tests;
