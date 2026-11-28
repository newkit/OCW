//! No plant on a road (the world-building rule of 2026-11-28: "tree on
//! road – this should never happen, in no world"): the one test every
//! plant of a package passes before it is placed – the grid's trees
//! ([`crate::trees`]), the mapped trees and the plantations' rows
//! ([`crate::plantings`]), the fields' stands and the renderer's own
//! placement – and the one the world builder's check (`world-builder
//! plants-check`, also run by `check`, `verify` and `publish`) runs over
//! everything a package would draw. One function in the crate both use, so
//! the build and the runtime placement cannot disagree.
//!
//! **The road surface** is what the game draws and rides: every road
//! strip of the plant's tile (carriageway, `width_m`, plus
//! [`EDGE_M`] for the kerb or the shoulder's lip) and every non-tunnel edge
//! of the road graph near it (the graph is whole in every app, also in a
//! streamed package, so a tile near a border sees the roads of the next
//! tile – the old placement only looked at the tile's own strips), all road
//! classes (tracks, paths and cycleways are drawn and ridden too),
//! roundabout rings, hairpins and ramps as their strips are drawn (after
//! the junction smoothing), bridges at their deck's height; and the
//! junction surfaces as the section `JNCT` stores their outlines (spec
//! 4.8.8: patches, clusters). Tunnels have no strips (the terrain above is
//! free). Sidewalks are not road surface: street trees may stand on them,
//! as in every town.
//!
//! **The rule** (for each surface piece, at the height of the road there):
//! - the **trunk** keeps [`TRUNK_CLEAR_M`] from the surface's edge;
//! - the **crown below the vehicle clearance** ([`VEHICLE_CLEARANCE_M`]
//!   above the road) keeps [`CROWN_CLEAR_M`] from the edge: nothing hangs
//!   low over a lane;
//! - **above the clearance** a crown may overhang the surface by at most
//!   [`OVERHANG_M`] (the canopy over a forest road);
//! - a plant whose top lies more than [`BELOW_M`] under the surface (under
//!   a viaduct) or a road more than the plant's height below its foot (in
//!   a cut) is no conflict.
//!
//! The crown's shape ([`PlantShape::of`]) follows the generator of
//! `ocw-render::flora` (cones of the spruces and firs from low down, the
//! broadleaves' ellipsoids, the pines' and palms' crowns up high, shrubs
//! from the ground) with a little more for its lumps.

use std::sync::{Arc, Mutex, Weak};

use crate::flora::{Habit, Species};
use crate::graph::{RoadClass, RoadGraph};
use crate::junction::section::{JunctionData, StoredRule};
use crate::package::{TileKey, WorldPackage};

/// Height above the road a crown must leave free over the lane (m): the
/// clearance of the vehicles (lorries, buses) under trees.
pub const VEHICLE_CLEARANCE_M: f64 = 4.5;
/// How far a crown above [`VEHICLE_CLEARANCE_M`] may reach over the road
/// surface's edge (m).
pub const OVERHANG_M: f64 = 2.0;
/// The drawn surface beyond the carriageway's half width (m): the kerb or
/// the shoulder's lip.
pub const EDGE_M: f64 = 0.3;
/// The trunk keeps this far from the surface's edge (m).
pub const TRUNK_CLEAR_M: f64 = 0.5;
/// A crown below the clearance keeps this far from the edge (m).
pub const CROWN_CLEAR_M: f64 = 0.2;
/// A plant whose top lies this far under a road surface passes under it
/// (a viaduct; m).
pub const BELOW_M: f64 = 0.5;
/// The widest need a plant can have (m): the space finds every surface
/// within this distance of a point (a large oak's crown at twice its
/// size).
pub const MAX_REACH_M: f64 = 20.0;
/// Grid cell of the space (m).
const CELL_M: f64 = 16.0;
/// A tile's strips reach this far into the next tile (m).
const TILE_REACH_M: f64 = 6.0;

/// How a crown is drawn: its widest radius and where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Crown {
    /// A cone from `start` (share of the height; widest there) to the top
    /// (spruces, firs, larches).
    Cone { start: f64 },
    /// An ellipsoid round `centre` reaching `half` up and down (shares of
    /// the height; broadleaves).
    Ellipsoid { centre: f64, half: f64 },
    /// As wide from `base` (share of the height) to the top (pines, palms,
    /// shrubs, hedges, stands).
    Column { base: f64 },
}

/// A plant as drawn, for the test: its height, trunk and crown (m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantShape {
    pub height_m: f64,
    /// The trunk's radius at the foot (0: no trunk).
    pub trunk_m: f64,
    /// The crown's widest radius.
    pub crown_m: f64,
    pub crown: Crown,
}

/// The crown's lumps and tiers reach a little beyond its radius.
const LUMPS: f64 = 1.1;

impl PlantShape {
    /// `sp` drawn at `scale` (the renderer's scale: height and crown both
    /// times it).
    pub fn of(sp: Species, scale: f32) -> Self {
        use Species::*;
        let s = f64::from(scale);
        let crown = match sp {
            NorwaySpruce => Crown::Cone { start: 0.1 },
            SilverFir => Crown::Cone { start: 0.16 },
            DouglasFir | Larch => Crown::Cone { start: 0.2 },
            ScotsPine | CanaryPine => Crown::Column { base: 0.4 },
            DatePalm | SlenderPalm => Crown::Column { base: 0.65 },
            DragonTree => Crown::Column { base: 0.45 },
            Banana => Crown::Column { base: 0.3 },
            _ => match sp.habit() {
                Habit::Deciduous | Habit::EvergreenBroadleaf => {
                    let (centre, half) = broadleaf_crown(sp);
                    Crown::Ellipsoid { centre, half }
                }
                _ => Crown::Column { base: 0.0 },
            },
        };
        Self {
            height_m: f64::from(sp.height_m()) * s,
            trunk_m: f64::from(sp.trunk_radius_m()) * s,
            crown_m: f64::from(sp.crown_radius_m()) * s * LUMPS,
            crown,
        }
    }

    /// A hedge, a vine row or a field's stand: `height_m` high, reaching
    /// `half_m` to the side from the ground up.
    pub fn hedge(height_m: f64, half_m: f64) -> Self {
        Self {
            height_m,
            trunk_m: 0.0,
            crown_m: half_m,
            crown: Crown::Column { base: 0.0 },
        }
    }

    /// The widest the crown reaches between `lo` and `hi` metres above the
    /// foot (0 where it has no crown).
    pub fn crown_within(&self, lo: f64, hi: f64) -> f64 {
        let h = self.height_m.max(1e-6);
        let (lo, hi) = (lo.max(0.0) / h, hi.min(h) / h);
        if hi < lo {
            return 0.0;
        }
        match self.crown {
            Crown::Cone { start } => {
                if hi < start {
                    0.0
                } else {
                    let y = lo.max(start);
                    self.crown_m * (1.0 - (y - start) / (1.0 - start).max(1e-6))
                }
            }
            Crown::Ellipsoid { centre, half } => {
                let y = centre.clamp(lo, hi);
                let t = (y - centre) / half.max(1e-6);
                if t.abs() >= 1.0 {
                    0.0
                } else {
                    self.crown_m * (1.0 - t * t).sqrt()
                }
            }
            Crown::Column { base } => {
                if hi < base {
                    0.0
                } else {
                    self.crown_m
                }
            }
        }
    }

    /// How far from a road surface's edge (m) the foot must stand when the
    /// surface lies `dy` metres above the foot (negative: below it);
    /// `None` when the plant passes under the road (a deck above its top).
    pub fn need(&self, dy: f64) -> Option<f64> {
        let top = self.height_m;
        if top < dy - BELOW_M {
            return None; // under the deck
        }
        let low = (dy - BELOW_M, dy + VEHICLE_CLEARANCE_M);
        let mut need: f64 = 0.0;
        if self.trunk_m > 0.0 && low.1 >= 0.0 {
            need = need.max(self.trunk_m + TRUNK_CLEAR_M);
        }
        if low.1 >= 0.0 {
            let c = self.crown_within(low.0, low.1);
            if c > 0.0 {
                need = need.max(c + CROWN_CLEAR_M);
            }
        }
        let high = self.crown_within(low.1.max(0.0), top) - OVERHANG_M;
        Some(need.max(high).min(MAX_REACH_M))
    }
}

/// The broadleaves' crowns (centre, half height as shares of the height):
/// `ocw-render::flora::broadleaf_shape`'s centre and vertical radius.
fn broadleaf_crown(sp: Species) -> (f64, f64) {
    use Species::*;
    match sp {
        Beech | SycamoreMaple => (0.62, 0.36),
        Ash => (0.66, 0.3),
        Oak => (0.6, 0.32),
        Alder => (0.58, 0.42),
        Willow => (0.58, 0.38),
        Birch => (0.64, 0.38),
        Unknown | Lime => (0.6, 0.42),
        FruitTree | Chestnut => (0.6, 0.38),
        Laurel => (0.55, 0.45),
        FayaHeath => (0.58, 0.42),
        Eucalyptus => (0.75, 0.24),
        SubtropicalFruit => (0.5, 0.48),
        _ => (0.6, 0.38),
    }
}

/// What a plant stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OnSurface {
    /// A carriageway (a strip or a graph edge) of this class.
    Road(RoadClass),
    /// A junction surface (spec 4.8.8).
    Junction,
}

impl OnSurface {
    pub fn label(self) -> String {
        match self {
            Self::Road(c) => format!("{} road", format!("{c:?}").to_lowercase()),
            Self::Junction => "junction".into(),
        }
    }
}

/// Which part of the plant is in the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Part {
    /// The trunk on (or too near) the surface.
    Trunk,
    /// A plant without a trunk (a stand, a vine row, a shrub) on it.
    Surface,
    /// The crown low over the lane (or hanging over it too far).
    Crown,
}

/// A plant on a road: what it stands on, which part, the nearest point of
/// the surface's centre line (x, z) and by how much (m) it is too near.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conflict {
    pub surface: OnSurface,
    pub part: Part,
    pub at: [f64; 2],
    pub by_m: f64,
    /// The foot's distance from the surface's edge (negative: on it).
    pub edge_m: f64,
}

#[derive(Debug, Clone, Copy)]
struct Seg {
    a: [f64; 2],
    b: [f64; 2],
    /// Heights at `a` and `b` (`NAN`: unknown, a conflict at any height).
    ya: f64,
    yb: f64,
    /// Half the drawn surface (carriageway + [`EDGE_M`]).
    half: f64,
    class: RoadClass,
}

#[derive(Debug, Clone)]
struct Patch {
    ring: Vec<[f64; 2]>,
    lo: [f64; 2],
    hi: [f64; 2],
    y: f64,
}

/// The graph's edges and junction surfaces by tile (made once per graph
/// and junction section, shared by every space of the package).
#[derive(Debug, Default)]
struct GraphIndex {
    tile_size: f64,
    segs: Vec<Seg>,
    patches: Vec<Patch>,
    /// Per tile: the segments and patches within [`MAX_REACH_M`] of it.
    tiles: std::collections::HashMap<TileKey, (Vec<u32>, Vec<u32>)>,
}

type CacheEntry = (Weak<RoadGraph>, Option<Weak<JunctionData>>, Arc<GraphIndex>);
static CACHE: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());

fn graph_index(pkg: &WorldPackage) -> Option<Arc<GraphIndex>> {
    let g = pkg.graph.as_ref()?;
    let tile_size = f64::from(pkg.tile_size_m);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(w, _, _)| w.strong_count() > 0);
    let same_j = |j: &Option<Weak<JunctionData>>| match (j, &pkg.junctions) {
        (None, None) => true,
        (Some(w), Some(a)) => std::ptr::eq(w.as_ptr(), Arc::as_ptr(a)),
        _ => false,
    };
    if let Some((_, _, ix)) = cache.iter().find(|(w, j, ix)| {
        std::ptr::eq(w.as_ptr(), Arc::as_ptr(g)) && same_j(j) && ix.tile_size == tile_size
    }) {
        return Some(ix.clone());
    }
    let ix = Arc::new(build_index(g, pkg.junctions.as_deref(), pkg, tile_size));
    cache.push((
        Arc::downgrade(g),
        pkg.junctions.as_ref().map(Arc::downgrade),
        ix.clone(),
    ));
    Some(ix)
}

fn build_index(
    g: &RoadGraph,
    junctions: Option<&JunctionData>,
    pkg: &WorldPackage,
    tile_size: f64,
) -> GraphIndex {
    let mut ix = GraphIndex {
        tile_size,
        ..Default::default()
    };
    let reach = MAX_REACH_M + TILE_REACH_M;
    let enter = |ix: &mut GraphIndex, lo: [f64; 2], hi: [f64; 2], item: u32, patch: bool| {
        let a = TileKey::containing([lo[0] - reach, lo[1] - reach], tile_size);
        let b = TileKey::containing([hi[0] + reach, hi[1] + reach], tile_size);
        for x in a.x..=b.x {
            for z in a.z..=b.z {
                let t = ix.tiles.entry(TileKey { x, z }).or_default();
                if patch { &mut t.1 } else { &mut t.0 }.push(item);
            }
        }
    };
    for e in &g.edges {
        if e.tunnel || e.points.len() < 2 {
            continue;
        }
        let half = e.width_m / 2.0 + EDGE_M;
        let hs = crate::lamps::edge_heights(e);
        for (i, w) in e.points.windows(2).enumerate() {
            let s = Seg {
                a: w[0],
                b: w[1],
                ya: hs.get(i).copied().unwrap_or(f64::NAN),
                yb: hs.get(i + 1).copied().unwrap_or(f64::NAN),
                half,
                class: e.class,
            };
            let lo = [w[0][0].min(w[1][0]) - half, w[0][1].min(w[1][1]) - half];
            let hi = [w[0][0].max(w[1][0]) + half, w[0][1].max(w[1][1]) + half];
            let id = ix.segs.len() as u32;
            ix.segs.push(s);
            enter(&mut ix, lo, hi, id, false);
        }
    }
    let index = junctions
        .and_then(|j| j.index.as_ref())
        .filter(|i| i.vertex_count as usize == g.vertices.len());
    if let Some(index) = index {
        // The road's height at each vertex: an edge's profile end there.
        let mut at_vertex = vec![f64::NAN; g.vertices.len()];
        for e in &g.edges {
            if let (Some(&a), Some(&b)) = (e.profile.first(), e.profile.last()) {
                for (v, y) in [(e.from.0 as usize, a), (e.to.0 as usize, b)] {
                    if let Some(h) = at_vertex.get_mut(v)
                        && h.is_nan()
                    {
                        *h = y;
                    }
                }
            }
        }
        for (v, rule) in &index.rules {
            let StoredRule::Patch(p) = rule else { continue };
            let Some(&at) = g.vertices.get(*v as usize) else {
                continue;
            };
            if p.outline.len() < 3 {
                continue;
            }
            let ring: Vec<[f64; 2]> = p
                .outline
                .iter()
                .map(|q| [at[0] + f64::from(q.d[0]), at[1] + f64::from(q.d[1])])
                .collect();
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for q in &ring {
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
            // The surface's height: the road at the vertex (the terrain is
            // settled just under it).
            let y = at_vertex
                .get(*v as usize)
                .copied()
                .filter(|y| !y.is_nan())
                .or_else(|| pkg.height(at[0], at[1]))
                .unwrap_or(f64::NAN);
            let id = ix.patches.len() as u32;
            ix.patches.push(Patch { ring, lo, hi, y });
            enter(&mut ix, lo, hi, id, true);
        }
    }
    ix
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64, [f64; 2]) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let c = [a[0] + dx * t, a[1] + dz * t];
    ((p[0] - c[0]).hypot(p[1] - c[1]), t, c)
}

fn inside(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut c = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            c = !c;
        }
    }
    c
}

/// The road surfaces round one tile: its own strips, the graph's edges and
/// the junction surfaces within [`MAX_REACH_M`] of it.
#[derive(Debug)]
pub struct RoadSpace {
    lo: [f64; 2],
    n: usize,
    cells: Vec<Vec<u32>>,
    segs: Vec<Seg>,
    graph: Option<Arc<GraphIndex>>,
    patches: Vec<u32>,
}

impl RoadSpace {
    /// The space for the plants of tile `key` (also a little beyond it:
    /// the neighbours' field cells at its edge).
    pub fn for_tile(pkg: &WorldPackage, key: TileKey) -> Self {
        let size = f64::from(pkg.tile_size_m);
        let o = key.origin(size);
        let margin = 20.0;
        let lo = [o[0] - margin, o[1] - margin];
        let n = ((size + 2.0 * margin) / CELL_M).ceil() as usize;
        let mut s = Self {
            lo,
            n,
            cells: vec![Vec::new(); n * n],
            segs: Vec::new(),
            graph: graph_index(pkg),
            patches: Vec::new(),
        };
        if let Some(tile) = pkg.tiles.get(&key) {
            for strip in &tile.roads {
                let half = f64::from(strip.width_m) / 2.0 + EDGE_M;
                for w in strip.points.windows(2) {
                    s.segs.push(Seg {
                        a: [f64::from(w[0][0]), f64::from(w[0][2])],
                        b: [f64::from(w[1][0]), f64::from(w[1][2])],
                        ya: f64::from(w[0][1]),
                        yb: f64::from(w[1][1]),
                        half,
                        class: strip.class,
                    });
                }
            }
        }
        if let Some(ix) = s.graph.clone()
            && let Some((segs, patches)) = ix.tiles.get(&key)
        {
            s.segs.extend(segs.iter().map(|&i| ix.segs[i as usize]));
            s.patches = patches.clone();
        }
        for i in 0..s.segs.len() {
            let sg = s.segs[i];
            s.enter_seg(sg.a, sg.b, sg.half + MAX_REACH_M, i as u32);
        }
        s
    }

    fn cell_of(&self, v: f64, lo: f64) -> isize {
        ((v - lo) / CELL_M).floor() as isize
    }

    fn enter_seg(&mut self, a: [f64; 2], b: [f64; 2], r: f64, item: u32) {
        let n = self.n as isize;
        let x0 = self.cell_of(a[0].min(b[0]) - r, self.lo[0]).max(0);
        let x1 = self.cell_of(a[0].max(b[0]) + r, self.lo[0]).min(n - 1);
        let z0 = self.cell_of(a[1].min(b[1]) - r, self.lo[1]).max(0);
        let z1 = self.cell_of(a[1].max(b[1]) + r, self.lo[1]).min(n - 1);
        let reach = r + CELL_M * std::f64::consts::FRAC_1_SQRT_2;
        for z in z0..=z1 {
            for x in x0..=x1 {
                let c = [
                    self.lo[0] + (x as f64 + 0.5) * CELL_M,
                    self.lo[1] + (z as f64 + 0.5) * CELL_M,
                ];
                if seg_dist(c, a, b).0 <= reach {
                    self.cells[z as usize * self.n + x as usize].push(item);
                }
            }
        }
    }

    fn items(&self, p: [f64; 2]) -> &[u32] {
        let (x, z) = (
            self.cell_of(p[0], self.lo[0]),
            self.cell_of(p[1], self.lo[1]),
        );
        let n = self.n as isize;
        if x < 0 || z < 0 || x >= n || z >= n {
            return &[];
        }
        &self.cells[z as usize * self.n + x as usize]
    }

    /// The worst conflict of a plant of `shape` standing at `foot` (x, y,
    /// z, package metres) with the road surfaces ([`PlantShape::need`]);
    /// `None`: it is off the road.
    pub fn conflict(&self, foot: [f64; 3], shape: &PlantShape) -> Option<Conflict> {
        let p = [foot[0], foot[2]];
        let mut worst: Option<Conflict> = None;
        let mut consider = |c: Conflict| {
            if worst.is_none_or(|w| c.by_m > w.by_m) {
                worst = Some(c);
            }
        };
        let judge = |edge: f64, ry: f64, surface: OnSurface, at: [f64; 2]| -> Option<Conflict> {
            // Unknown height: as if level with the foot.
            let need = shape.need(if ry.is_nan() { 0.0 } else { ry - foot[1] })?;
            if edge >= need {
                return None;
            }
            let trunk = shape.trunk_m > 0.0 && edge < shape.trunk_m + TRUNK_CLEAR_M;
            Some(Conflict {
                surface,
                part: if trunk {
                    Part::Trunk
                } else if edge < 0.0 {
                    Part::Surface
                } else {
                    Part::Crown
                },
                at,
                by_m: need - edge,
                edge_m: edge,
            })
        };
        for &i in self.items(p) {
            let s = &self.segs[i as usize];
            let (d, t, c) = seg_dist(p, s.a, s.b);
            let edge = d - s.half;
            if edge >= MAX_REACH_M {
                continue;
            }
            let ry = s.ya + (s.yb - s.ya) * t;
            if let Some(c) = judge(edge, ry, OnSurface::Road(s.class), c) {
                consider(c);
            }
        }
        if let Some(ix) = &self.graph {
            for &i in &self.patches {
                let pa = &ix.patches[i as usize];
                if p[0] < pa.lo[0] - MAX_REACH_M
                    || p[0] > pa.hi[0] + MAX_REACH_M
                    || p[1] < pa.lo[1] - MAX_REACH_M
                    || p[1] > pa.hi[1] + MAX_REACH_M
                {
                    continue;
                }
                let n = pa.ring.len();
                let (mut d, mut at) = (f64::MAX, pa.ring[0]);
                for k in 0..n {
                    let (dk, _, ck) = seg_dist(p, pa.ring[k], pa.ring[(k + 1) % n]);
                    if dk < d {
                        (d, at) = (dk, ck);
                    }
                }
                let edge = if inside(&pa.ring, p) { -d } else { d };
                if let Some(c) = judge(edge, pa.y, OnSurface::Junction, at) {
                    consider(c);
                }
            }
        }
        worst
    }

    /// Whether a plant of `shape` may stand at `foot`.
    pub fn clear(&self, foot: [f64; 3], shape: &PlantShape) -> bool {
        self.conflict(foot, shape).is_none()
    }

    /// Whether anything of the road lies within `r` metres of (x, z) at
    /// any height (a cheap first test: plants farther away are clear).
    pub fn near(&self, p: [f64; 2], r: f64) -> bool {
        self.items(p).iter().any(|&i| {
            let s = &self.segs[i as usize];
            seg_dist(p, s.a, s.b).0 - s.half < r
        }) || self.graph.as_ref().is_some_and(|ix| {
            self.patches.iter().any(|&i| {
                let pa = &ix.patches[i as usize];
                p[0] > pa.lo[0] - r
                    && p[0] < pa.hi[0] + r
                    && p[1] > pa.lo[1] - r
                    && p[1] < pa.hi[1] + r
            })
        })
    }
}

// --- The check -----------------------------------------------------------------

/// Where a plant comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// The tree grid by land use and measured cover ([`crate::trees`]).
    Grid,
    /// The provisional patches of packages without land use.
    Placeholder,
    /// An OSM `natural=tree` (section `VEGE`).
    MappedTree,
    /// A tree of an OSM `natural=tree_row`.
    TreeRow,
    /// A plantation's plant in a row (fruit trees, bananas, avocados).
    RowPlant,
    /// A vine or berry row.
    VineRow,
    /// A field's stand (maize, cereals, …).
    Stand,
    /// A mapped tree as the section `VEGE` stores it (also one the
    /// placement leaves out: older clients draw what is stored).
    StoredTree,
    /// A trunk the section `OBST` stores (the server's and the
    /// prediction's trees beside the roads: a tree cell or a planted tree).
    StoredTrunk,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Grid => "tree grid",
            Self::Placeholder => "placeholder trees",
            Self::MappedTree => "mapped tree",
            Self::TreeRow => "tree row",
            Self::RowPlant => "plantation row plant",
            Self::VineRow => "vine row",
            Self::Stand => "field stand",
            Self::StoredTree => "stored tree (VEGE)",
            Self::StoredTrunk => "stored trunk (OBST)",
        }
    }
}

impl Part {
    pub fn label(self) -> &'static str {
        match self {
            Self::Trunk => "trunk",
            Self::Surface => "on it",
            Self::Crown => "crown",
        }
    }
}

/// A plant as a package draws it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub source: Source,
    pub foot: [f64; 3],
    pub shape: PlantShape,
    pub species: Option<Species>,
}

/// The vine rows' height and half width (m; `ocw-render::vegetation`).
pub fn vine_shape(region: crate::flora::FloraRegion) -> PlantShape {
    let h = if region == crate::flora::FloraRegion::Tenerife {
        1.1
    } else {
        1.6
    };
    PlantShape::hedge(h, 0.4)
}

/// The tallest field stand (maize, m).
pub const STAND_M: f64 = 2.5;

/// The points of a field cell (centre `c`, side `cs`) the test looks at:
/// its corners, edge middles and centre.
pub fn cell_points(c: [f64; 2], cs: f64) -> [[f64; 2]; 9] {
    let h = cs / 2.0;
    let mut out = [[0.0; 2]; 9];
    let mut i = 0;
    for dz in [-h, 0.0, h] {
        for dx in [-h, 0.0, h] {
            out[i] = [c[0] + dx, c[1] + dz];
            i += 1;
        }
    }
    out
}

/// Every plant tile `key` of `pkg` draws, as the renderer places it
/// (`ocw-render::vegetation::tile_plants`: the same sources, species and
/// scales; the near and far models stand at the same feet): the grid's
/// trees, the mapped trees, the plantations' rows (vine rows every metre)
/// and the fields' stands (each cell's [`cell_points`]).
pub fn tile_plants(pkg: &WorldPackage, key: TileKey) -> Vec<Placed> {
    let mut out = Vec::new();
    let Some(tile) = pkg.tiles.get(&key) else {
        return out;
    };
    let gt = crate::flora::GridTrees::of_tile(pkg, key);
    let f = |p: [f32; 3]| p.map(f64::from);
    for ((p, s), class) in crate::trees::classed_tree_sites(pkg, tile) {
        let (sp, scale) = gt.tree(pkg, class, p, s);
        out.push(Placed {
            source: if class.is_some() {
                Source::Grid
            } else {
                Source::Placeholder
            },
            foot: f(p),
            shape: PlantShape::of(sp, scale),
            species: Some(sp),
        });
    }
    for (foot, t) in crate::plantings::mapped_trees(pkg, key) {
        let (sp, scale) = gt.mapped(pkg, foot, &t);
        out.push(Placed {
            source: match t.source {
                crate::vegetation::TreeSource::Tree => Source::MappedTree,
                crate::vegetation::TreeSource::TreeRow => Source::TreeRow,
            },
            foot: f(foot),
            shape: PlantShape::of(sp, scale),
            species: Some(sp),
        });
    }
    let (runs, plants) = crate::plantings::tile_rows(pkg, key);
    for p in plants {
        out.push(Placed {
            source: Source::RowPlant,
            foot: f(p.pos),
            shape: PlantShape::of(p.species, p.scale * 0.8),
            species: Some(p.species),
        });
    }
    let vine = vine_shape(gt.region);
    for r in runs {
        let (a, b) = (r.a.map(f64::from), r.b.map(f64::from));
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let n = len.ceil().max(1.0) as usize;
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let (x, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            let Some(y) = pkg.height(x, z) else { continue };
            out.push(Placed {
                source: Source::VineRow,
                foot: [x, y, z],
                shape: vine,
                species: None,
            });
        }
    }
    for (c, cs) in crate::plantings::stand_cells(pkg, key) {
        for [x, z] in cell_points(c, cs) {
            let Some(y) = pkg.height(x, z) else { continue };
            out.push(Placed {
                source: Source::Stand,
                foot: [x, y, z],
                shape: PlantShape::hedge(STAND_M, 0.0),
                species: None,
            });
        }
    }
    out
}

/// A stored trunk: only its centre counts (within [`TRUNK_CLEAR_M`] of the
/// surface's edge or on it).
fn trunk_only(height_m: f64) -> PlantShape {
    PlantShape {
        height_m,
        trunk_m: 1e-3,
        crown_m: 0.0,
        crown: Crown::Column { base: 1.0 },
    }
}

/// What tile `key` of `pkg` stores of its trees: the mapped trees of
/// `VEGE` as stored and the trunks of `OBST` (tree cells, planted trees).
pub fn tile_stored(pkg: &WorldPackage, key: TileKey) -> Vec<Placed> {
    let mut out = Vec::new();
    let gt = crate::flora::GridTrees::of_tile(pkg, key);
    if let Some(v) = pkg.look.vegetation.as_ref().and_then(|v| v.tiles.get(&key)) {
        for t in &v.trees {
            let Some(y) = pkg.height(f64::from(t.pos[0]), f64::from(t.pos[1])) else {
                continue;
            };
            let foot = [t.pos[0], y as f32 - 0.3, t.pos[1]];
            let (sp, scale) = gt.mapped(pkg, foot, t);
            out.push(Placed {
                source: Source::StoredTree,
                foot: foot.map(f64::from),
                shape: PlantShape::of(sp, scale),
                species: Some(sp),
            });
        }
    }
    if let Some(o) = pkg.obstacles.as_ref().and_then(|o| o.tiles.get(&key)) {
        let side = crate::trees::cells_per_side(pkg.tile_size_m);
        let mut trunks: Vec<([f64; 2], f64)> = o
            .tree_cells(side)
            .map(|c| crate::trees::trunks_from_cells(key, pkg.tile_size_m, &c))
            .unwrap_or_default()
            .into_iter()
            .map(|(p, _)| (p, 10.0))
            .collect();
        trunks.extend(
            o.points
                .iter()
                .filter(|p| p.kind == crate::obstacles::ObstacleKind::PlantedTree)
                .map(|p| (p.pos.map(f64::from), f64::from(p.height_m))),
        );
        for (p, h) in trunks {
            let Some(y) = pkg.height(p[0], p[1]) else {
                continue;
            };
            out.push(Placed {
                source: Source::StoredTrunk,
                foot: [p[0], y, p[1]],
                shape: trunk_only(h),
                species: None,
            });
        }
    }
    out
}

/// A plant on a road: the plant and its conflict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Violation {
    pub tile: TileKey,
    pub plant: Placed,
    pub conflict: Conflict,
}

/// The plants on the roads of tile `key` ([`tile_plants`] and
/// [`tile_stored`] against [`RoadSpace::for_tile`]) and how many plants
/// it has.
pub fn tile_violations(pkg: &WorldPackage, key: TileKey) -> (usize, Vec<Violation>) {
    let space = RoadSpace::for_tile(pkg, key);
    let mut plants = tile_plants(pkg, key);
    plants.extend(tile_stored(pkg, key));
    let n = plants.len();
    let v = plants
        .into_iter()
        .filter_map(|plant| {
            space
                .conflict(plant.foot, &plant.shape)
                .map(|conflict| Violation {
                    tile: key,
                    plant,
                    conflict,
                })
        })
        .collect();
    (n, v)
}

/// What the check found in a package.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// Plants tested.
    pub plants: usize,
    /// Plants on a road, in tile order (all of them).
    pub violations: Vec<Violation>,
}

impl Report {
    /// The count per (source, part).
    pub fn by_source(&self) -> std::collections::BTreeMap<(Source, Part), usize> {
        let mut m = std::collections::BTreeMap::new();
        for v in &self.violations {
            *m.entry((v.plant.source, v.conflict.part)).or_default() += 1;
        }
        m
    }

    /// The count per surface kind.
    pub fn by_surface(&self) -> std::collections::BTreeMap<OnSurface, usize> {
        let mut m = std::collections::BTreeMap::new();
        for v in &self.violations {
            *m.entry(v.conflict.surface).or_default() += 1;
        }
        m
    }
}

/// Checks every plant of every tile of `pkg` (on all cores natively).
pub fn check_package(pkg: &WorldPackage) -> Report {
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let one = |key: TileKey| tile_violations(pkg, key);
    let mut r = Report::default();
    for (n, v) in crate::lamps::each_tile(&keys, &one) {
        r.plants += n;
        r.violations.extend(v);
    }
    r
}

#[doc(hidden)]
pub mod fixture;

#[cfg(test)]
mod tests;
