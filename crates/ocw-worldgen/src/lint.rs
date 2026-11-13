//! World lint (spec §4.8.1): finds and counts what looks physically wrong
//! in a built world package – steps and kinks in the roads, roads floating
//! above or buried in the terrain, strips of neighbouring roads at different
//! heights, near-miss road ends, rivers flowing uphill, tilted lakes,
//! floating or buried buildings and land-use slivers. Pure: it reads only
//! the package (and, for the river and lake checks, the water bodies of the
//! OSM extract). `world-builder lint` prints the report.

use std::collections::BTreeMap;

use ocw_world::graph::{PROFILE_STEP_M, RoadGraph};
use ocw_world::landuse::LandClass;
use ocw_world::package::{TileKey, WorldPackage};

use crate::profile::limits;
use crate::spatial::SegmentIndex;
use crate::water::WaterBodies;

/// Worst cases kept per check.
pub const WORST: usize = 8;
/// Sample spacing along roads (m).
pub const ROAD_STEP_M: f64 = 2.0;
/// A road floats where the terrain is more than this below its surface
/// anywhere across it (the builder leaves 0.3 m, spec §4.3).
pub const FLOAT_M: f64 = 0.8;
/// Strips that overlap with more than this height difference show a step.
pub const STEP_M: f64 = 0.25;
/// A dead end this close to another road was meant to join it.
pub const NEAR_MISS_M: f64 = 3.0;

/// One finding: how bad, where (package metres), which OSM object.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub value: f64,
    pub at: [f64; 2],
    /// OSM way id (negative: relation), 0 if none.
    pub osm: i64,
    pub note: String,
}

/// Count, extent and worst cases of one check.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tally {
    pub count: usize,
    /// Metres of road (or square metres of area, or cells) affected.
    pub amount: f64,
    pub worst: Vec<Case>,
}

impl Tally {
    pub fn add(&mut self, amount: f64, case: Case) {
        self.count += 1;
        self.amount += amount;
        if self.worst.len() < WORST || case.value > self.worst[WORST - 1].value {
            let at = self
                .worst
                .iter()
                .position(|c| c.value < case.value)
                .unwrap_or(self.worst.len());
            self.worst.insert(at, case);
            self.worst.truncate(WORST);
        }
    }
}

/// The lint report: tallies by check name (`road.float.untagged.1-2m`, …).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LintReport {
    pub road_m: f64,
    pub bridge_m: f64,
    pub tallies: BTreeMap<String, Tally>,
}

impl LintReport {
    fn add(&mut self, key: &str, amount: f64, case: Case) {
        self.tallies
            .entry(key.to_string())
            .or_default()
            .add(amount, case);
    }

    /// The tally of a check (empty if nothing was found).
    pub fn get(&self, key: &str) -> Tally {
        self.tallies.get(key).cloned().unwrap_or_default()
    }

    pub fn count(&self, key: &str) -> usize {
        self.tallies.get(key).map_or(0, |t| t.count)
    }

    /// Sum of the amounts of all checks starting with `prefix`.
    pub fn amount_with_prefix(&self, prefix: &str) -> f64 {
        self.tallies
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(_, t)| t.amount)
            .sum()
    }
}

/// Profile height at arc length `d` (as `Edge::height_at`, with the length
/// given).
fn profile_at(p: &[f64], len: f64, d: f64) -> f64 {
    if p.len() < 2 {
        return p.first().copied().unwrap_or(0.0);
    }
    let d = d.clamp(0.0, len);
    let i = ((d / PROFILE_STEP_M).floor() as usize).min(p.len() - 2);
    let seg = (len - i as f64 * PROFILE_STEP_M).clamp(1e-9, PROFILE_STEP_M);
    let t = ((d - i as f64 * PROFILE_STEP_M) / seg).clamp(0.0, 1.0);
    p[i] + (p[i + 1] - p[i]) * t
}

/// Points every `step` m along a polyline with arc length and direction.
fn walk(points: &[[f64; 2]], step: f64) -> Vec<([f64; 2], f64, [f64; 2])> {
    let mut out = Vec::new();
    let mut walked = 0.0;
    let mut next = step / 2.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len < 1e-9 {
            continue;
        }
        let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        while next <= walked + len {
            let t = (next - walked) / len;
            out.push((
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                next,
                dir,
            ));
            next += step;
        }
        walked += len;
    }
    out
}

fn turn_deg(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
    let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
    if lu < 1e-9 || lv < 1e-9 {
        return 0.0;
    }
    ((u[0] * v[0] + u[1] * v[1]) / (lu * lv))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

fn segments_cross(p1: [f64; 2], p2: [f64; 2], q1: [f64; 2], q2: [f64; 2]) -> bool {
    let c = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let (d1, d2) = (c(q1, q2, p1), c(q1, q2, p2));
    let (d3, d4) = (c(p1, p2, q1), c(p1, p2, q2));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// A spike: an interior point where the line turns back (more than this)
/// with a leg shorter than [`SPIKE_LEG_M`].
pub const SPIKE_TURN_DEG: f64 = 150.0;
pub const SPIKE_LEG_M: f64 = 15.0;
/// Consecutive points closer than this are duplicates.
pub const DUPLICATE_M: f64 = 0.25;

/// Horizontal checks of one polyline: (duplicates, spike points, self-crossings).
pub fn polyline_faults(points: &[[f64; 2]]) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let mut dups = Vec::new();
    for (i, w) in points.windows(2).enumerate() {
        if (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) < DUPLICATE_M {
            dups.push(i + 1);
        }
    }
    let mut spikes = Vec::new();
    for i in 1..points.len().saturating_sub(1) {
        let (a, b, c) = (points[i - 1], points[i], points[i + 1]);
        let (la, lc) = (
            (b[0] - a[0]).hypot(b[1] - a[1]),
            (c[0] - b[0]).hypot(c[1] - b[1]),
        );
        if la < DUPLICATE_M || lc < DUPLICATE_M {
            continue;
        }
        if turn_deg(a, b, c) > SPIKE_TURN_DEG && la.min(lc) < SPIKE_LEG_M {
            spikes.push(i);
        }
    }
    let mut crossings = Vec::new();
    if points.len() <= 400 {
        for i in 0..points.len().saturating_sub(1) {
            for j in i + 2..points.len() - 1 {
                if segments_cross(points[i], points[i + 1], points[j], points[j + 1]) {
                    crossings.push(i);
                }
            }
        }
    }
    (dups, spikes, crossings)
}

/// Height bands of floating roads (m of terrain below the road surface).
fn float_band(gap: f64) -> &'static str {
    match gap {
        g if g < 1.5 => "0.8-1.5m",
        g if g < 3.0 => "1.5-3m",
        g if g < 6.0 => "3-6m",
        _ => ">6m",
    }
}

/// Global terrain vertex → water level, for the cause of a floating road.
struct WaterVertices<'a> {
    pkg: &'a WorldPackage,
}

impl WaterVertices<'_> {
    /// Whether any corner of the terrain cell under `p` has a water level.
    fn near_water(&self, p: [f64; 2]) -> bool {
        let Some(lu) = &self.pkg.landuse else {
            return false;
        };
        let size = f64::from(self.pkg.tile_size_m);
        let key = TileKey::containing(p, size);
        let Some(t) = lu.tiles.get(&key) else {
            return false;
        };
        if t.water.is_empty() {
            return false;
        }
        let o = key.origin(size);
        let n = self.pkg.samples as usize;
        let sp = self.pkg.spacing_m();
        let (i, k) = (
            (((p[0] - o[0]) / sp) as usize).min(n - 2),
            (((p[1] - o[1]) / sp) as usize).min(n - 2),
        );
        [(0, 0), (1, 0), (0, 1), (1, 1)]
            .iter()
            .any(|&(di, dk)| t.water_level(((k + dk) * n + i + di) as u32).is_some())
    }
}

/// Bridge decks of the package (section `BRDG`) by 50 m bucket.
struct Decks<'a> {
    buckets: std::collections::HashMap<(i64, i64), Vec<&'a ocw_world::bridges::Bridge>>,
}

impl<'a> Decks<'a> {
    fn new(pkg: &'a WorldPackage) -> Self {
        let mut buckets: std::collections::HashMap<(i64, i64), Vec<&ocw_world::bridges::Bridge>> =
            Default::default();
        for b in pkg.bridges.iter().flat_map(|t| t.values()).flatten() {
            let mut keys: Vec<(i64, i64)> = b
                .points
                .iter()
                .map(|p| {
                    (
                        (f64::from(p[0]) / 50.0).floor() as i64,
                        (f64::from(p[2]) / 50.0).floor() as i64,
                    )
                })
                .collect();
            keys.sort_unstable();
            keys.dedup();
            for k in keys {
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        buckets.entry((k.0 + dx, k.1 + dz)).or_default().push(b);
                    }
                }
            }
        }
        Self { buckets }
    }

    /// Whether a road point at height `y` lies on a bridge deck.
    fn on_deck(&self, p: [f64; 2], y: f64) -> bool {
        let k = ((p[0] / 50.0).floor() as i64, (p[1] / 50.0).floor() as i64);
        self.buckets.get(&k).is_some_and(|v| {
            v.iter().any(|b| {
                b.deck_near(p[0] as f32, p[1] as f32)
                    .is_some_and(|(d, dy)| {
                        d <= b.width_m / 2.0 + 0.5 && (f64::from(dy) - y).abs() < 0.3
                    })
            })
        })
    }
}

/// Runs every check on `pkg`; `water` adds the river and lake checks.
pub fn lint(pkg: &WorldPackage, water: Option<&WaterBodies>) -> LintReport {
    let mut r = LintReport::default();
    if let Some(g) = &pkg.graph {
        lint_roads(pkg, g, &mut r);
    }
    lint_water_grid(pkg, &mut r);
    if let Some(w) = water {
        lint_rivers_and_lakes(pkg, w, &mut r);
    }
    lint_buildings(pkg, &mut r);
    lint_landuse(pkg, &mut r);
    r
}

fn case(at: [f64; 2], value: f64, osm: i64, note: String) -> Case {
    Case {
        value,
        at,
        osm,
        note,
    }
}

fn lint_roads(pkg: &WorldPackage, g: &RoadGraph, r: &mut LintReport) {
    let index = SegmentIndex::new(g, 20.0, |e| !g.edges[e].tunnel);
    let adj = g.adjacency();
    let lens: Vec<f64> = g.edges.iter().map(|e| e.length_m()).collect();
    let water = WaterVertices { pkg };
    let decks = Decks::new(pkg);
    for b in pkg.bridges.iter().flat_map(|t| t.values()).flatten() {
        r.add(
            &format!("bridge.{:?}.{:?}", b.source, b.structure).to_lowercase(),
            f64::from(b.length_m()),
            case(
                [f64::from(b.points[0][0]), f64::from(b.points[0][2])],
                f64::from(b.length_m()),
                b.osm_way,
                format!(
                    "{:?}, {} piers, {} arches",
                    b.material,
                    b.piers.len(),
                    b.arches.len()
                ),
            ),
        );
    }
    let max_half = g.edges.iter().map(|e| e.width_m / 2.0).fold(0.0, f64::max);
    for (ei, e) in g.edges.iter().enumerate() {
        let len = lens[ei];
        if e.bridge {
            r.bridge_m += len;
        } else if !e.tunnel {
            r.road_m += len;
        }
        // Horizontal faults.
        let (dups, spikes, crossings) = polyline_faults(&e.points);
        for i in dups {
            r.add(
                "road.horizontal.duplicate_node",
                0.0,
                case(e.points[i], 1.0, e.osm_way, String::new()),
            );
        }
        for i in spikes {
            let (a, b, c) = (e.points[i - 1], e.points[i], e.points[i + 1]);
            let leg = (b[0] - a[0])
                .hypot(b[1] - a[1])
                .min((c[0] - b[0]).hypot(c[1] - b[1]));
            r.add(
                "road.horizontal.spike",
                leg,
                case(b, turn_deg(a, b, c), e.osm_way, format!("leg {leg:.1} m")),
            );
        }
        for i in crossings {
            r.add(
                "road.horizontal.self_crossing",
                0.0,
                case(e.points[i], 1.0, e.osm_way, String::new()),
            );
        }
        // Vertical: grade, steps and kinks of the profile.
        let p = &e.profile;
        if p.len() >= 2 {
            let (max_grade, _) = limits(e.class);
            let d: Vec<f64> = (0..p.len())
                .map(|i| (i as f64 * PROFILE_STEP_M).min(len))
                .collect();
            let mut prev_g: Option<f64> = None;
            for i in 0..p.len() - 1 {
                let ds = d[i + 1] - d[i];
                if ds < 1.0 {
                    continue;
                }
                let grade = (p[i + 1] - p[i]) / ds;
                let at = point_at(&e.points, d[i]);
                if grade.abs() > max_grade + 0.01 && !e.bridge && !e.tunnel {
                    r.add(
                        "road.vertical.grade_over_limit",
                        ds,
                        case(
                            at,
                            grade.abs() * 100.0,
                            e.osm_way,
                            format!("{:?}, limit {:.0} %", e.class, max_grade * 100.0),
                        ),
                    );
                }
                if (p[i + 1] - p[i]).abs() > 1.0 && grade.abs() > max_grade + 0.05 {
                    r.add(
                        "road.vertical.step",
                        ds,
                        case(
                            at,
                            (p[i + 1] - p[i]).abs(),
                            e.osm_way,
                            format!("{:?}", e.class),
                        ),
                    );
                }
                if let Some(pg) = prev_g
                    && (grade - pg).abs() > 0.08
                {
                    r.add(
                        "road.vertical.kink",
                        ds,
                        case(
                            at,
                            (grade - pg).abs() * 100.0,
                            e.osm_way,
                            format!("{:+.1} % → {:+.1} %", pg * 100.0, grade * 100.0),
                        ),
                    );
                }
                prev_g = Some(grade);
            }
        }
        if e.tunnel || p.is_empty() {
            continue;
        }
        let half = e.width_m / 2.0;
        for (c, s, dir) in walk(&e.points, ROAD_STEP_M) {
            let y = profile_at(p, len, s);
            let on_bridge = e.bridge || decks.on_deck(c, y);
            let kind = if on_bridge { "bridge" } else { "untagged" };
            // Surface against the terrain, across the road.
            let (mut max_gap, mut min_gap) = (f64::NEG_INFINITY, f64::INFINITY);
            for off in [-(half - 0.25).max(0.0), 0.0, (half - 0.25).max(0.0)] {
                let q = [c[0] - dir[1] * off, c[1] + dir[0] * off];
                if let Some(t) = pkg.surface_height(q[0], q[1]) {
                    max_gap = max_gap.max(y - t);
                    min_gap = min_gap.min(y - t);
                }
            }
            if max_gap.is_finite() {
                if min_gap < -0.05 {
                    r.add(
                        &format!("road.buried.{kind}"),
                        ROAD_STEP_M,
                        case(c, -min_gap, e.osm_way, format!("{:?}", e.class)),
                    );
                }
                if max_gap > FLOAT_M {
                    r.add(
                        &format!("road.float.{kind}.{}", float_band(max_gap)),
                        ROAD_STEP_M,
                        case(
                            c,
                            max_gap,
                            e.osm_way,
                            format!("{:?} {}", e.class, e.name.as_deref().unwrap_or("")),
                        ),
                    );
                    if !on_bridge {
                        let cause = if water.near_water(c) {
                            "water"
                        } else if index.near(g, c, max_half + 15.0).iter().any(|h| {
                            let o = &g.edges[h.edge];
                            h.edge != ei
                                && !o.bridge
                                && h.dist <= o.width_m / 2.0 + 15.0
                                && !o.profile.is_empty()
                                && profile_at(&o.profile, lens[h.edge], h.along) < y - 0.5
                        }) {
                            "lower_road_nearby"
                        } else {
                            "other"
                        };
                        r.add(
                            &format!("road.float_cause.{cause}"),
                            ROAD_STEP_M,
                            case(c, max_gap, e.osm_way, String::new()),
                        );
                    }
                }
            }
            // Untagged road over water (a missing bridge).
            if !on_bridge
                && let Some(lu) = &pkg.landuse
                && lu.class_at(f64::from(pkg.tile_size_m), c[0], c[1]) == Some(LandClass::Water)
            {
                r.add(
                    "road.over_water_untagged",
                    ROAD_STEP_M,
                    case(c, 1.0, e.osm_way, format!("{:?}", e.class)),
                );
            }
            // Other strips under this point at a different height.
            for h in index.near(g, c, max_half) {
                let o = &g.edges[h.edge];
                if h.edge == ei
                    || o.tunnel
                    || o.profile.is_empty()
                    || h.dist > o.width_m / 2.0 - 0.25
                {
                    continue;
                }
                let dh = (profile_at(&o.profile, lens[h.edge], h.along) - y).abs();
                if dh <= STEP_M {
                    continue;
                }
                let shares = [e.from, e.to].iter().any(|v| *v == o.from || *v == o.to);
                if shares {
                    r.add(
                        "junction.surface_step",
                        ROAD_STEP_M,
                        case(c, dh, e.osm_way, format!("on way {}", o.osm_way)),
                    );
                } else if !(e.bridge || o.bridge) {
                    r.add(
                        "road.crossing_step_untagged",
                        ROAD_STEP_M,
                        case(c, dh, e.osm_way, format!("over/under way {}", o.osm_way)),
                    );
                }
            }
        }
    }
    // Junctions: the profile ends of all roads meeting there.
    for (v, edges) in adj.iter().enumerate() {
        let ends: Vec<f64> = edges
            .iter()
            .filter_map(|&ei| {
                let e = &g.edges[ei];
                let p = &e.profile;
                if p.is_empty() {
                    return None;
                }
                Some(if e.from.0 as usize == v {
                    p[0]
                } else {
                    p[p.len() - 1]
                })
            })
            .collect();
        if ends.len() >= 2 {
            let lo = ends.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = ends.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if hi - lo > 0.05 {
                r.add(
                    "junction.height_mismatch",
                    0.0,
                    case(g.vertices[v], hi - lo, 0, String::new()),
                );
            }
        }
        // Dead ends that nearly touch another road.
        if edges.len() == 1
            && !g
                .boundary_vertices
                .contains(&ocw_world::graph::VertexId(v as u32))
        {
            let own = &g.edges[edges[0]];
            let pv = g.vertices[v];
            let yv = if own.from.0 as usize == v {
                own.profile.first()
            } else {
                own.profile.last()
            };
            if let Some(h) = index.near(g, pv, NEAR_MISS_M).into_iter().find(|h| {
                let o = &g.edges[h.edge];
                h.edge != edges[0]
                    && o.from.0 as usize != v
                    && o.to.0 as usize != v
                    && h.dist > 0.05
            }) {
                let o = &g.edges[h.edge];
                let dh = match yv {
                    Some(&y) if !o.profile.is_empty() => {
                        (profile_at(&o.profile, lens[h.edge], h.along) - y).abs()
                    }
                    _ => 0.0,
                };
                r.add(
                    "road.near_miss_end",
                    0.0,
                    case(
                        pv,
                        h.dist,
                        own.osm_way,
                        format!(
                            "{:.1} m from way {}, {dh:.1} m apart in height",
                            h.dist, o.osm_way
                        ),
                    ),
                );
            }
        }
    }
}

fn point_at(points: &[[f64; 2]], d: f64) -> [f64; 2] {
    let mut walked = 0.0;
    for w in points.windows(2) {
        let len = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if walked + len >= d && len > 0.0 {
            let t = ((d - walked) / len).clamp(0.0, 1.0);
            return [
                w[0][0] + (w[1][0] - w[0][0]) * t,
                w[0][1] + (w[1][1] - w[0][1]) * t,
            ];
        }
        walked += len;
    }
    *points.last().expect("non-empty")
}

/// Neighbouring water vertices whose levels differ visibly.
fn lint_water_grid(pkg: &WorldPackage, r: &mut LintReport) {
    let Some(lu) = &pkg.landuse else {
        return;
    };
    let n = pkg.samples as usize;
    let sp = pkg.spacing_m();
    let size = f64::from(pkg.tile_size_m);
    for (key, t) in &lu.tiles {
        if t.water.is_empty() {
            continue;
        }
        let o = key.origin(size);
        let levels: BTreeMap<u32, f32> = t.water.iter().copied().collect();
        for (&idx, &l) in &levels {
            let (i, k) = (idx as usize % n, idx as usize / n);
            for (di, dk) in [(1, 0), (0, 1)] {
                if i + di >= n || k + dk >= n {
                    continue;
                }
                let j = ((k + dk) * n + i + di) as u32;
                if let Some(&m) = levels.get(&j) {
                    let dl = f64::from((l - m).abs());
                    if dl > 0.5 {
                        r.add(
                            "water.level_step",
                            1.0,
                            case(
                                [o[0] + i as f64 * sp, o[1] + k as f64 * sp],
                                dl,
                                0,
                                String::new(),
                            ),
                        );
                    }
                }
            }
        }
    }
}

/// The water level stored at the grid vertex nearest to `p`.
fn level_near(pkg: &WorldPackage, p: [f64; 2]) -> Option<f64> {
    let lu = pkg.landuse.as_ref()?;
    let size = f64::from(pkg.tile_size_m);
    let key = TileKey::containing(p, size);
    let t = lu.tiles.get(&key)?;
    let o = key.origin(size);
    let n = pkg.samples as usize;
    let sp = pkg.spacing_m();
    let i = (((p[0] - o[0]) / sp).round() as usize).min(n - 1);
    let k = (((p[1] - o[1]) / sp).round() as usize).min(n - 1);
    t.water_level((k * n + i) as u32).map(f64::from)
}

fn lint_rivers_and_lakes(pkg: &WorldPackage, w: &WaterBodies, r: &mut LintReport) {
    for chain in crate::water::river_chains(&w.rivers) {
        let width = chain
            .iter()
            .map(|&i| w.rivers[i].width_m)
            .fold(0.0, f64::max);
        if width < crate::landuse::SURFACE_MIN_WIDTH_M {
            continue;
        }
        let mut line: Vec<[f64; 2]> = Vec::new();
        for &i in &chain {
            let skip = usize::from(!line.is_empty());
            line.extend_from_slice(&w.rivers[i].points[skip..]);
        }
        let osm = w.rivers[chain[0]].osm_way;
        let mut lowest = f64::INFINITY;
        let mut prev: Option<f64> = None;
        for p in crate::profile::resample(&line, crate::water::RIVER_STEP_M) {
            let Some(l) = level_near(pkg, p) else {
                prev = None;
                continue;
            };
            if l > lowest + 0.3 {
                r.add(
                    "water.river_uphill",
                    crate::water::RIVER_STEP_M,
                    case(p, l - lowest, osm, String::new()),
                );
            }
            if let Some(q) = prev
                && (l - q).abs() > 0.5
            {
                r.add(
                    "water.river_step",
                    crate::water::RIVER_STEP_M,
                    case(p, (l - q).abs(), osm, String::new()),
                );
            }
            lowest = lowest.min(l);
            prev = Some(l);
        }
    }
    let sp = pkg.spacing_m();
    for lake in &w.lakes {
        let b = lake.bbox;
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut at = [0.0, 0.0];
        let mut z = (b[1] / sp).ceil() * sp;
        while z <= b[3] {
            let mut x = (b[0] / sp).ceil() * sp;
            while x <= b[2] {
                if lake.contains([x, z])
                    && let Some(l) = level_near(pkg, [x, z])
                {
                    lo = lo.min(l);
                    if l > hi {
                        hi = l;
                        at = [x, z];
                    }
                }
                x += sp;
            }
            z += sp;
        }
        if hi - lo > 0.25 {
            r.add(
                "water.lake_not_level",
                1.0,
                case(at, hi - lo, lake.osm_id, String::new()),
            );
        }
    }
}

fn lint_buildings(pkg: &WorldPackage, r: &mut LintReport) {
    let Some(b) = &pkg.buildings else {
        return;
    };
    for bd in b.values().flatten() {
        let mut pts: Vec<[f64; 2]> = bd
            .outline
            .iter()
            .map(|p| [f64::from(p[0]), f64::from(p[1])])
            .collect();
        let n = pts.len().max(1) as f64;
        let c = pts
            .iter()
            .fold([0.0, 0.0], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
        pts.push(c);
        let hs: Vec<f64> = pts
            .iter()
            .filter_map(|p| pkg.surface_height(p[0], p[1]))
            .collect();
        if hs.is_empty() {
            continue;
        }
        let lo = hs.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = hs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let (base, eave) = (f64::from(bd.base_y), f64::from(bd.eave_y));
        if lo < base - 0.02 {
            r.add(
                "building.floating",
                1.0,
                case(c, base - lo, 0, format!("{:?}", bd.kind)),
            );
        }
        if hi > base + 0.5 * (eave - base) {
            r.add(
                "building.half_buried",
                1.0,
                case(
                    c,
                    hi - base,
                    0,
                    format!("{:?}, wall {:.1} m", bd.kind, eave - base),
                ),
            );
        }
        if hi - lo > 3.0 {
            r.add(
                "building.on_steep_slope",
                1.0,
                case(c, hi - lo, 0, format!("{:?}", bd.kind)),
            );
        }
    }
}

/// Components up to this many cells count as slivers (5 m cells: 50 m²).
pub const SLIVER_CELLS: usize = 2;
/// Unclassified holes up to this many cells between areas count as gaps.
pub const GAP_CELLS: usize = 6;

/// Small connected components of one class in a cell grid (4-neighbour),
/// not touching the grid border: (class, cells, neighbour classes).
pub fn small_components(
    classes: &[LandClass],
    n: usize,
    max: usize,
) -> Vec<(LandClass, Vec<usize>, Vec<LandClass>)> {
    let mut seen = vec![false; classes.len()];
    let mut out = Vec::new();
    for start in 0..classes.len() {
        if seen[start] {
            continue;
        }
        let class = classes[start];
        let mut stack = vec![start];
        seen[start] = true;
        let mut cells = Vec::new();
        let mut border = false;
        let mut around: Vec<LandClass> = Vec::new();
        while let Some(c) = stack.pop() {
            cells.push(c);
            let (i, k) = (c % n, c / n);
            if i == 0 || k == 0 || i == n - 1 || k == n - 1 {
                border = true;
            }
            let mut nb = Vec::with_capacity(4);
            if i > 0 {
                nb.push(c - 1);
            }
            if i + 1 < n {
                nb.push(c + 1);
            }
            if k > 0 {
                nb.push(c - n);
            }
            if k + 1 < n {
                nb.push(c + n);
            }
            for d in nb {
                if classes[d] == class {
                    if !seen[d] {
                        seen[d] = true;
                        stack.push(d);
                    }
                } else if !around.contains(&classes[d]) {
                    around.push(classes[d]);
                }
            }
        }
        if !border && cells.len() <= max {
            out.push((class, cells, around));
        }
    }
    out
}

fn lint_landuse(pkg: &WorldPackage, r: &mut LintReport) {
    let Some(lu) = &pkg.landuse else {
        return;
    };
    let n = lu.cells as usize;
    let size = f64::from(pkg.tile_size_m);
    let cell = size / n as f64;
    for (key, t) in &lu.tiles {
        let o = key.origin(size);
        for (class, cells, around) in small_components(&t.classes, n, GAP_CELLS) {
            let c = cells[0];
            let at = [
                o[0] + ((c % n) as f64 + 0.5) * cell,
                o[1] + ((c / n) as f64 + 0.5) * cell,
            ];
            let area = cells.len() as f64 * cell * cell;
            if class == LandClass::Default {
                if around.len() == 1 {
                    r.add(
                        "landuse.gap",
                        area,
                        case(at, cells.len() as f64, 0, format!("in {:?}", around[0])),
                    );
                }
            } else if class != LandClass::Water && cells.len() <= SLIVER_CELLS {
                r.add(
                    "landuse.sliver",
                    area,
                    case(
                        at,
                        cells.len() as f64,
                        0,
                        format!("{class:?} in {around:?}"),
                    ),
                );
            }
        }
    }
}

/// The report as text: one line per check with count, amount and the worst
/// cases with latitude / longitude.
pub fn format_report(r: &LintReport, origin: (f64, f64)) -> String {
    use std::fmt::Write;
    let proj = ocw_world::geo::LocalProjection::new(origin.0, origin.1);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "roads: {:.1} km (+ {:.1} km bridges)",
        r.road_m / 1000.0,
        r.bridge_m / 1000.0
    );
    for (k, t) in &r.tallies {
        let amount = if k.starts_with("road.")
            || k.starts_with("junction.surface")
            || k.starts_with("water.river")
        {
            format!(", {:.0} m", t.amount)
        } else if k.starts_with("landuse") {
            format!(", {:.0} m²", t.amount)
        } else {
            String::new()
        };
        let share = if k.starts_with("road.float")
            || k.starts_with("road.buried")
            || k.starts_with("road.over_water")
        {
            let total = if k.contains(".bridge") {
                r.bridge_m
            } else {
                r.road_m
            };
            format!(
                " ({:.3} % of {})",
                100.0 * t.amount / total.max(1.0),
                if k.contains(".bridge") {
                    "bridges"
                } else {
                    "roads"
                }
            )
        } else {
            String::new()
        };
        let _ = writeln!(s, "{k}: {}{amount}{share}", t.count);
        for c in t.worst.iter().take(3) {
            let (lat, lon) = proj.unproject(c.at);
            let osm = if c.osm > 0 {
                format!(" way {}", c.osm)
            } else if c.osm < 0 {
                format!(" relation {}", -c.osm)
            } else {
                String::new()
            };
            let _ = writeln!(
                s,
                "    {:.2} at {:.0},{:.0} ({lat:.5},{lon:.5}){osm} {}",
                c.value, c.at[0], c.at[1], c.note
            );
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::buildings::{Building, BuildingKind, Roof};
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, VertexId};
    use ocw_world::landuse::{LandUse, LandUseTile};
    use ocw_world::package::Tile;

    fn edge(
        points: Vec<[f64; 2]>,
        from: u32,
        to: u32,
        class: RoadClass,
        profile: Vec<f64>,
    ) -> Edge {
        Edge {
            id: EdgeId(u64::from(from) * 100 + u64::from(to)),
            from: VertexId(from),
            to: VertexId(to),
            points,
            class,
            name: None,
            width_m: class.default_width_m(),
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: i64::from(from) + 1,
            profile,
        }
    }

    /// Flat terrain at 100 m; a primary road along z = 500 at 100.3 m (as
    /// the builder leaves it), a side road rising 1 m in its first 5 m, a
    /// dead end 2 m short of the primary, a floating track and a profile
    /// with a 3 m step.
    fn pkg() -> WorldPackage {
        let n = 101usize;
        let key = TileKey { x: 0, z: 0 };
        let mut tiles = std::collections::BTreeMap::new();
        tiles.insert(
            key,
            Tile {
                key,
                heights: vec![100.0; n * n],
                roads: vec![],
            },
        );
        let main = edge(
            vec![[100.0, 500.0], [500.0, 500.0], [900.0, 500.0]],
            0,
            1,
            RoadClass::Primary,
            vec![100.3; 161],
        );
        let side = edge(
            vec![[500.0, 500.0], [500.0, 300.0]],
            1,
            2,
            RoadClass::Residential,
            (0..41)
                .map(|i| if i == 0 { 100.3 } else { 101.3 })
                .collect(),
        );
        let near_miss = edge(
            vec![[300.0, 502.0], [300.0, 700.0]],
            3,
            4,
            RoadClass::Residential,
            vec![100.3; 41],
        );
        let floating = edge(
            vec![[700.0, 800.0], [800.0, 800.0]],
            5,
            6,
            RoadClass::Track,
            vec![102.0; 21],
        );
        let mut step = vec![100.3; 21];
        for h in step.iter_mut().skip(10) {
            *h += 3.0;
        }
        let stepped = edge(
            vec![
                [100.0, 900.0],
                [150.0, 900.0],
                [150.05, 900.0],
                [200.0, 900.0],
            ],
            7,
            8,
            RoadClass::Residential,
            step,
        );
        let g = RoadGraph {
            vertices: vec![
                [100.0, 500.0],
                [500.0, 500.0],
                [500.0, 300.0],
                [300.0, 502.0],
                [300.0, 700.0],
                [700.0, 800.0],
                [800.0, 800.0],
                [100.0, 900.0],
                [200.0, 900.0],
            ],
            edges: vec![main, side, near_miss, floating, stepped],
            boundary_vertices: Default::default(),
        };
        let mut classes = vec![LandClass::Forest; 200 * 200];
        classes[50 * 200 + 50] = LandClass::Meadow;
        WorldPackage {
            name: "t".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: n as u32,
            tiles,
            graph: Some(g),
            landuse: Some(LandUse {
                cells: 200,
                tiles: [(
                    key,
                    LandUseTile {
                        classes,
                        water: vec![],
                    },
                )]
                .into(),
            }),
            buildings: Some(
                [(
                    key,
                    vec![Building {
                        kind: BuildingKind::House,
                        outline: vec![
                            [600.0, 600.0],
                            [610.0, 600.0],
                            [610.0, 610.0],
                            [600.0, 610.0],
                        ],
                        holes: vec![],
                        base_y: 101.0,
                        eave_y: 107.0,
                        roof: Roof::Flat,
                    }],
                )]
                .into(),
            ),
            bridges: None,
            routes: vec![],
            region: None,
        }
    }

    #[test]
    fn the_lint_finds_what_looks_wrong() {
        let r = lint(&pkg(), None);
        // The track floats 2 m over 100 m.
        let f = r.get("road.float.untagged.1.5-3m");
        assert!((100.0..=106.0).contains(&f.amount), "{f:?}");
        assert!(r.get("road.float_cause.other").count >= f.count);
        // The side road floats too, beside the lower primary.
        assert!(r.count("road.float_cause.lower_road_nearby") > 0);
        assert_eq!(r.count("road.buried.untagged"), 0);
        // The 3 m step: over the limit, a step and two kinks (and the side
        // road's kink where it levels out).
        assert_eq!(r.count("road.vertical.step"), 1);
        assert!(r.count("road.vertical.grade_over_limit") >= 1);
        assert_eq!(r.count("road.vertical.kink"), 3);
        // The duplicate node at 150.05 and the dead end 2 m off the primary.
        assert_eq!(r.count("road.horizontal.duplicate_node"), 1);
        let nm = r.get("road.near_miss_end");
        assert_eq!(nm.count, 1);
        assert!((nm.worst[0].value - 2.0).abs() < 1e-6);
        // The side road stands above the primary's surface near the junction.
        assert!(r.count("junction.surface_step") >= 1);
        // The house floats 1 m over the ground; the meadow cell is a sliver.
        assert_eq!(r.count("building.floating"), 1);
        assert_eq!(r.count("landuse.sliver"), 1);
        let text = format_report(&r, (47.0, 9.0));
        assert!(text.contains("road.near_miss_end: 1"), "{text}");
    }

    #[test]
    fn tallies_keep_the_worst_cases_in_order() {
        let mut t = Tally::default();
        for v in [3.0, 1.0, 9.0, 4.0, 7.0, 2.0, 8.0, 5.0, 6.0, 0.5] {
            t.add(
                1.0,
                Case {
                    value: v,
                    at: [0.0, 0.0],
                    osm: 0,
                    note: String::new(),
                },
            );
        }
        assert_eq!(t.count, 10);
        assert_eq!(t.worst.len(), WORST);
        let v: Vec<f64> = t.worst.iter().map(|c| c.value).collect();
        assert_eq!(v, vec![9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0]);
    }

    #[test]
    fn small_components_ignore_the_tile_edge() {
        let n = 5;
        let mut c = vec![LandClass::Forest; n * n];
        c[0] = LandClass::Meadow;
        c[2 * n + 2] = LandClass::Meadow;
        let small = small_components(&c, n, 2);
        assert_eq!(small.len(), 1);
        assert_eq!(small[0].1, vec![2 * n + 2]);
        assert_eq!(small[0].2, vec![LandClass::Forest]);
    }
}
