//! Where a street lamp's foot may stand (the world look's *Street lamps*
//! rule, 2026-11-22: "do not put lights/lantern poles in the road, only at
//! the side"): never on a carriageway – the lamp's own road's or any other
//! road's, roundabout rings, parking aisles and the cycle lanes painted on
//! them included – nor on a junction surface (patches and clusters, spec
//! §4.8.8), nor in a building, on a wall, or in the air beside a bridge
//! deck or through one. Only the arm and the head reach over the road.
//!
//! A [`LampSpace`] holds what stands round one package tile: the road graph's
//! carriageways (every road, not only the tile's strips, so a lamp near a
//! tile border sees the roads of the next tile – the graph is whole in
//! both apps, also in a streamed package), the tile's own strips (packages
//! without a graph), the junction surfaces of the vertices nearby, and the
//! buildings, walls and bridge decks of the tile and its eight neighbours
//! (what a streamed package keeps resident round a tile, [`crate::package::resident_cut`]).
//! It is a pure function of the package, so the desktop app, the browser
//! app and the world lint place the same lamps. The point obstacles
//! (bollards, posts: OSM `barrier=*` nodes, which stand in roads on
//! purpose) are not part of it and never move.

use crate::bridges::{KERB_M, PARAPET_T_M};
use crate::graph::RoadGraph;
use crate::junction::{JunctionSet, Patch};
use crate::obstacles::ObstacleKind;
use crate::package::{TileKey, WorldPackage};

/// Half the width of a lamp pole (m; the files' poles are 0.14 m square).
pub const POLE_R_M: f64 = 0.07;
/// Lamps are placed with the pole's centre at least this far outside a
/// carriageway's edge and a junction surface's outline (m).
pub const FOOT_CLEAR_M: f64 = 0.3;
/// A road or junction surface this far above or below the foot does not
/// block it (a road under a bridge, a bridge over a road; m).
pub const LEVEL_M: f64 = 3.0;
/// A bridge deck more than this above or below the foot is in the pole's
/// way (or the foot hangs beside it; m).
pub const DECK_LEVEL_M: f64 = 1.0;
/// How far round the tile the space reaches (m): lamps stand at most
/// 12 m from their strip, whose points reach 5 m into the next tile.
const MARGIN_M: f64 = 40.0;
/// Junction surfaces reach at most this far from their vertex (m; fork
/// mouths at 40 m, corner arcs).
const PATCH_REACH_M: f64 = 60.0;
/// Grid cell (m).
const CELL_M: f64 = 16.0;
/// Items are entered into the cells within this distance of them (m).
const EXPAND_M: f64 = FOOT_CLEAR_M + 0.2;

/// What a lamp foot would stand in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Blocked {
    /// A carriageway (with its painted cycle lanes and crossings).
    Carriageway,
    /// A junction surface (patch or cluster, spec §4.8.8).
    Junction,
    Building,
    Wall,
    /// Beside a bridge deck in the air, or through a deck above.
    Deck,
}

impl Blocked {
    pub fn label(self) -> &'static str {
        match self {
            Self::Carriageway => "carriageway",
            Self::Junction => "junction",
            Self::Building => "building",
            Self::Wall => "wall",
            Self::Deck => "deck",
        }
    }
}

/// A carriageway piece: centre line from `a` to `b` with its heights
/// (`NAN`: unknown, blocks at any height).
#[derive(Debug, Clone, Copy)]
struct Road {
    a: [f64; 2],
    b: [f64; 2],
    ya: f64,
    yb: f64,
    half: f64,
    bridge: bool,
}

#[derive(Debug, Clone, Copy)]
struct Deck {
    a: [f64; 3],
    b: [f64; 3],
    /// Half the carriageway on it (the deck is [`KERB_M`] wider each side).
    half: f64,
}

#[derive(Debug, Clone, Copy)]
enum Item {
    Road(u32),
    Patch(u32),
    Building(u32),
    Wall(u32),
    Deck(u32),
}

/// What stands round one tile, for placing its lamps.
pub struct LampSpace<'a> {
    lo: [f64; 2],
    n: usize,
    cells: Vec<Vec<Item>>,
    roads: Vec<Road>,
    patches: Vec<&'a Patch>,
    buildings: Vec<&'a crate::buildings::Building>,
    walls: Vec<([f64; 2], [f64; 2])>,
    decks: Vec<Deck>,
}

/// Distance from `p` to the segment `a`–`b` and the share along it.
fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dz * t), t)
}

fn inside(ring: &[[f32; 2]], p: [f64; 2]) -> bool {
    let mut c = false;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i].map(f64::from);
        let b = ring[(i + n - 1) % n].map(f64::from);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            c = !c;
        }
    }
    c
}

fn ring_dist(ring: &[[f32; 2]], p: [f64; 2]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| seg_dist(p, ring[i].map(f64::from), ring[(i + 1) % n].map(f64::from)).0)
        .fold(f64::MAX, f64::min)
}

/// The keys of a tile and its eight neighbours.
pub fn ring(key: TileKey) -> impl Iterator<Item = TileKey> {
    (-1..=1).flat_map(move |dz| {
        (-1..=1).map(move |dx| TileKey {
            x: key.x + dx,
            z: key.z + dz,
        })
    })
}

/// The graph edges listed for a tile and its eight neighbours (`edges`:
/// the edges reaching into a tile, e.g.
/// [`crate::junction::section::tile_edges`]), sorted and unique.
pub fn ring_edges<'e>(key: TileKey, edges: impl Fn(TileKey) -> &'e [u32]) -> Vec<u32> {
    let mut out: Vec<u32> = ring(key).flat_map(|k| edges(k).iter().copied()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

impl<'a> LampSpace<'a> {
    /// The space round tile `key`: the carriageways of the graph `edges`
    /// near it ([`ring_edges`]) and of the tile's own strips, the junction
    /// surfaces of `junctions` at their vertices, the buildings, walls and
    /// bridge decks of the tile and its neighbours.
    pub fn new(
        pkg: &'a WorldPackage,
        key: TileKey,
        edges: &[u32],
        junctions: Option<&'a JunctionSet>,
    ) -> Self {
        let size = f64::from(pkg.tile_size_m);
        let o = key.origin(size);
        let lo = [o[0] - MARGIN_M, o[1] - MARGIN_M];
        let n = ((size + 2.0 * MARGIN_M) / CELL_M).ceil() as usize;
        let mut s = Self {
            lo,
            n,
            cells: vec![Vec::new(); n * n],
            roads: Vec::new(),
            patches: Vec::new(),
            buildings: Vec::new(),
            walls: Vec::new(),
            decks: Vec::new(),
        };
        let hi = [lo[0] + n as f64 * CELL_M, lo[1] + n as f64 * CELL_M];
        let near = |a: [f64; 2], b: [f64; 2], r: f64| {
            a[0].min(b[0]) - r < hi[0]
                && a[0].max(b[0]) + r > lo[0]
                && a[1].min(b[1]) - r < hi[1]
                && a[1].max(b[1]) + r > lo[1]
        };
        // The tile's own strips (also without a graph).
        if let Some(tile) = pkg.tiles.get(&key) {
            for strip in &tile.roads {
                let half = f64::from(strip.width_m) / 2.0;
                for w in strip.points.windows(2) {
                    let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
                    s.add_road(Road {
                        a: [a[0], a[2]],
                        b: [b[0], b[2]],
                        ya: a[1],
                        yb: b[1],
                        half,
                        bridge: false,
                    });
                }
            }
        }
        // Every road of the graph near the tile, and its junctions.
        if let Some(g) = pkg.graph.as_deref() {
            let mut vertices = Vec::new();
            for &ei in edges {
                let Some(e) = g.edges.get(ei as usize) else {
                    continue;
                };
                if e.tunnel || e.points.len() < 2 {
                    continue;
                }
                let half = e.width_m / 2.0;
                let heights = edge_heights(e);
                for (i, w) in e.points.windows(2).enumerate() {
                    if !near(w[0], w[1], half + EXPAND_M) {
                        continue;
                    }
                    s.add_road(Road {
                        a: w[0],
                        b: w[1],
                        ya: heights.get(i).copied().unwrap_or(f64::NAN),
                        yb: heights.get(i + 1).copied().unwrap_or(f64::NAN),
                        half,
                        bridge: e.bridge,
                    });
                }
                vertices.extend([e.from.0, e.to.0]);
            }
            if let Some(js) = junctions {
                s.add_patches(g, js, vertices, near);
            }
        }
        for k in ring(key) {
            if let Some(list) = pkg.buildings.as_ref().and_then(|b| b.get(&k)) {
                for b in list {
                    if b.outline.len() < 3 {
                        continue;
                    }
                    let (mut bl, mut bh) = ([f64::MAX; 2], [f64::MIN; 2]);
                    for p in &b.outline {
                        bl = [bl[0].min(f64::from(p[0])), bl[1].min(f64::from(p[1]))];
                        bh = [bh[0].max(f64::from(p[0])), bh[1].max(f64::from(p[1]))];
                    }
                    if !near(bl, bh, EXPAND_M) {
                        continue;
                    }
                    let i = s.buildings.len() as u32;
                    s.buildings.push(b);
                    s.enter(bl, bh, EXPAND_M, Item::Building(i));
                }
            }
            if let Some(t) = pkg.obstacles.as_ref().and_then(|o| o.tiles.get(&k)) {
                for l in t.lines.iter().filter(|l| l.kind == ObstacleKind::Wall) {
                    for w in l.points.windows(2) {
                        let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
                        if !near(a, b, EXPAND_M) {
                            continue;
                        }
                        let i = s.walls.len() as u32;
                        s.walls.push((a, b));
                        s.enter_seg(a, b, EXPAND_M, Item::Wall(i));
                    }
                }
            }
            if let Some(list) = pkg.bridges.as_ref().and_then(|b| b.get(&k)) {
                for br in list {
                    let half = f64::from(br.width_m) / 2.0;
                    for w in br.points.windows(2) {
                        let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
                        let r = half + f64::from(KERB_M) + EXPAND_M;
                        if !near([a[0], a[2]], [b[0], b[2]], r) {
                            continue;
                        }
                        let i = s.decks.len() as u32;
                        s.decks.push(Deck { a, b, half });
                        s.enter_seg([a[0], a[2]], [b[0], b[2]], r, Item::Deck(i));
                    }
                }
            }
        }
        s
    }

    fn add_patches(
        &mut self,
        g: &RoadGraph,
        js: &'a JunctionSet,
        mut vertices: Vec<u32>,
        near: impl Fn([f64; 2], [f64; 2], f64) -> bool,
    ) {
        vertices.sort_unstable();
        vertices.dedup();
        let mut seen = std::collections::BTreeSet::new();
        for v in vertices {
            let Some(&at) = g.vertices.get(v as usize) else {
                continue;
            };
            if !near(at, at, PATCH_REACH_M) {
                continue;
            }
            let Some(p) = js.patch(g, v as usize) else {
                continue;
            };
            if !seen.insert(p.vertex) {
                continue;
            }
            let i = self.patches.len() as u32;
            self.patches.push(p);
            self.enter(p.min, p.max, EXPAND_M, Item::Patch(i));
        }
    }

    fn add_road(&mut self, r: Road) {
        let i = self.roads.len() as u32;
        self.roads.push(r);
        self.enter_seg(r.a, r.b, r.half + EXPAND_M, Item::Road(i));
    }

    fn cell_range(&self, v: f64, lo: f64) -> isize {
        ((v - lo) / CELL_M).floor() as isize
    }

    fn enter(&mut self, a: [f64; 2], b: [f64; 2], r: f64, item: Item) {
        let n = self.n as isize;
        let x0 = self.cell_range(a[0].min(b[0]) - r, self.lo[0]).max(0);
        let x1 = self.cell_range(a[0].max(b[0]) + r, self.lo[0]).min(n - 1);
        let z0 = self.cell_range(a[1].min(b[1]) - r, self.lo[1]).max(0);
        let z1 = self.cell_range(a[1].max(b[1]) + r, self.lo[1]).min(n - 1);
        for z in z0..=z1 {
            for x in x0..=x1 {
                self.cells[z as usize * self.n + x as usize].push(item);
            }
        }
    }

    /// A segment's cells: those of its bounding box within `r` of it.
    fn enter_seg(&mut self, a: [f64; 2], b: [f64; 2], r: f64, item: Item) {
        let n = self.n as isize;
        let x0 = self.cell_range(a[0].min(b[0]) - r, self.lo[0]).max(0);
        let x1 = self.cell_range(a[0].max(b[0]) + r, self.lo[0]).min(n - 1);
        let z0 = self.cell_range(a[1].min(b[1]) - r, self.lo[1]).max(0);
        let z1 = self.cell_range(a[1].max(b[1]) + r, self.lo[1]).min(n - 1);
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

    fn items(&self, p: [f64; 2]) -> &[Item] {
        let x = self.cell_range(p[0], self.lo[0]);
        let z = self.cell_range(p[1], self.lo[1]);
        let n = self.n as isize;
        if x < 0 || z < 0 || x >= n || z >= n {
            return &[];
        }
        &self.cells[z as usize * self.n + x as usize]
    }

    /// The road surface `p` (at height `y`) lies on, `clear` metres round
    /// it included, and how deep (m) – the carriageways and junction
    /// surfaces at about the foot's level ([`LEVEL_M`]).
    pub fn road_at(&self, p: [f64; 2], y: f64, clear: f64) -> Option<(Blocked, f64)> {
        let mut worst: Option<(Blocked, f64)> = None;
        for item in self.items(p) {
            match *item {
                Item::Road(i) => {
                    let r = &self.roads[i as usize];
                    let (d, t) = seg_dist(p, r.a, r.b);
                    let ry = r.ya + (r.yb - r.ya) * t;
                    if d < r.half + clear && (ry.is_nan() || (ry - y).abs() <= LEVEL_M) {
                        let depth = r.half + clear - d;
                        if worst.is_none_or(|(_, w)| depth > w) {
                            worst = Some((Blocked::Carriageway, depth));
                        }
                    }
                }
                Item::Patch(i) if worst.is_none() => {
                    let pa = self.patches[i as usize];
                    let probes = [
                        [0.0, 0.0],
                        [clear, 0.0],
                        [-clear, 0.0],
                        [0.0, clear],
                        [0.0, -clear],
                    ];
                    let hit = probes.iter().any(|d| {
                        pa.height_at([p[0] + d[0], p[1] + d[1]])
                            .is_some_and(|h| (h - y).abs() <= LEVEL_M)
                    });
                    if hit {
                        worst = Some((Blocked::Junction, clear));
                    }
                }
                _ => {}
            }
        }
        worst
    }

    /// Whether a lamp foot at `foot` (x, y, z) is blocked, with the pole's
    /// centre kept [`FOOT_CLEAR_M`] from every road surface.
    pub fn blocked(&self, foot: [f64; 3]) -> Option<Blocked> {
        let p = [foot[0], foot[2]];
        if let Some((b, _)) = self.road_at(p, foot[1], FOOT_CLEAR_M) {
            return Some(b);
        }
        for item in self.items(p) {
            match *item {
                Item::Building(i) => {
                    let b = self.buildings[i as usize];
                    let inside_walls =
                        inside(&b.outline, p) && !b.holes.iter().any(|h| inside(h, p));
                    if inside_walls || ring_dist(&b.outline, p) < POLE_R_M + 0.1 {
                        return Some(Blocked::Building);
                    }
                }
                Item::Wall(i) => {
                    let (a, b) = self.walls[i as usize];
                    if seg_dist(p, a, b).0 < POLE_R_M + 0.2 {
                        return Some(Blocked::Wall);
                    }
                }
                Item::Deck(i) => {
                    let d = &self.decks[i as usize];
                    let (dist, t) = seg_dist(p, [d.a[0], d.a[2]], [d.b[0], d.b[2]]);
                    let dy = d.a[1] + (d.b[1] - d.a[1]) * t - foot[1];
                    let edge = d.half + f64::from(KERB_M);
                    // Through a deck above or below, or hanging off its
                    // side at its height.
                    let beside = dist < edge + 1.0 && dist >= edge && dy.abs() <= DECK_LEVEL_M;
                    if (dist < edge + POLE_R_M && dy.abs() > DECK_LEVEL_M) || beside {
                        return Some(Blocked::Deck);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The bridge deck a road point `p` (at height `y`) lies on: the
    /// distance from its centre line to where a lamp stands on it (on
    /// the parapet or the rail, past the kerb).
    pub fn deck_foot(&self, p: [f64; 2], y: f64) -> Option<f64> {
        self.items(p).iter().find_map(|item| match *item {
            Item::Deck(i) => {
                let d = &self.decks[i as usize];
                let (dist, t) = seg_dist(p, [d.a[0], d.a[2]], [d.b[0], d.b[2]]);
                let dy = d.a[1] + (d.b[1] - d.a[1]) * t - y;
                (dist < d.half.max(0.5) && dy.abs() < 0.5)
                    .then(|| d.half + f64::from(KERB_M) - f64::from(PARAPET_T_M) / 2.0)
            }
            _ => None,
        })
    }

    /// Whether a road point lies on a bridge of the graph (an edge tagged
    /// `bridge`, deck or not).
    pub fn on_bridge(&self, p: [f64; 2]) -> bool {
        self.items(p).iter().any(|item| match *item {
            Item::Road(i) => {
                let r = &self.roads[i as usize];
                r.bridge && seg_dist(p, r.a, r.b).0 < 0.5
            }
            _ => false,
        })
    }
}

/// The edge's road heights at its points (empty without a profile).
fn edge_heights(e: &crate::graph::Edge) -> Vec<f64> {
    use crate::graph::PROFILE_STEP_M;
    let p = &e.profile;
    if p.is_empty() {
        return Vec::new();
    }
    let len = e.length_m();
    let mut s = 0.0;
    let mut out = Vec::with_capacity(e.points.len());
    for (i, q) in e.points.iter().enumerate() {
        if i > 0 {
            let a = e.points[i - 1];
            s += (q[0] - a[0]).hypot(q[1] - a[1]);
        }
        let d = s.clamp(0.0, len);
        let f = d / PROFILE_STEP_M;
        let k = (f.floor() as usize).min(p.len() - 1);
        out.push(if k + 1 >= p.len() {
            p[p.len() - 1]
        } else {
            let seg = (len - k as f64 * PROFILE_STEP_M).clamp(1e-9, PROFILE_STEP_M);
            let t = ((d - k as f64 * PROFILE_STEP_M) / seg).clamp(0.0, 1.0);
            p[k] + (p[k + 1] - p[k]) * t
        });
    }
    out
}
