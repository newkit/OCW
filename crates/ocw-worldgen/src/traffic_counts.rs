//! Open traffic counts into the package (spec 14 Phase 18, 18B *Demand*,
//! 18D, 18E T11): counting stations of the open sources (the Cabildo de
//! Tenerife's IMD, Liechtenstein's permanent stations, the Swiss ASTRA
//! counts, Baden-Württemberg's SVZ) matched to the road-graph edges, and
//! a per-edge **flow factor** – the counted annual average daily traffic
//! against the class's default the demand model uses – written into
//! `TRFC`'s `flow`. The sim scales spawn rates and route choice by it
//! (`ocw_sim::traffic::world::edge_rule`).
//!
//! **Input:** one CSV per source in the OCW counts format, written by
//! `tools/traffic-counts/fetch_counts.py` from the downloaded open data
//! (cached in the world's data folder, never fetched by the builder):
//!
//! ```text
//! # ocw-traffic-counts 1
//! # source: cabildo-tenerife-imd          (a short id)
//! # title: …   # licence: …   # notice: …  (the attribution, verbatim)
//! # url: …     # year: 2025
//! id,name,lat,lon,ref,aadt,aadt_dir1,aadt_dir2,dir1_bearing_deg
//! ```
//!
//! `aadt` is the annual average daily traffic of the cross-section (both
//! directions, motor vehicles a day); `aadt_dir1/2` the directions where
//! known, `dir1_bearing_deg` the compass bearing of direction 1 (empty
//! when unknown).
//!
//! **Matching** ([`match_stations`]): every motor edge within the
//! tolerance of a station (2.5 × for an edge with the station's road
//! number: wide motorways) is a candidate; the score is the distance plus
//! penalties: a different road number (`ref`) rules an edge out (reported
//! when no other is near), an edge without a number costs 30 m, a small
//! road class costs more (service roads 40 m, residential 15 m), an edge
//! whose name is in the station's name (Liechtenstein's "Vaduz
//! Zollstrasse") gains 10 m. On a dual carriageway the station's
//! directions go to the two one-way carriageways (by the bearing; half
//! each without). Reported: stations without an edge (with the nearest
//! motor edge), a road number that does not match, two roads within 8 m
//! of score (ambiguous: the best is used), several stations on one edge
//! (their mean is used).
//!
//! **Propagation** ([`flow_factors`]): from each matched station along the
//! same road (the same number, else the same name, else through vertices
//! without a junction) with the weight `exp(-d / L) · 0.8^junctions` (`L`
//! by the class of the counted road: 10 km trunk, 6 km primary, 4 km
//! secondary, 2.5 km tertiary, 1.2 km below), cut below 0.03; an edge's
//! volume is the weighted mean of the stations reaching it and the class
//! default (weight [`PRIOR_WEIGHT`]), so it falls back to the default far
//! from any station and is interpolated between two. The counted edges
//! themselves get their count. Deterministic: no hash order reaches the
//! result.

use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use ocw_world::geo::LocalProjection;
use ocw_world::graph::{Oneway, RoadClass, RoadGraph};
use ocw_world::manifest::Attribution;
use ocw_world::traffic::TrafficData;

/// The first line of a counts file.
pub const COUNTS_MAGIC: &str = "# ocw-traffic-counts 1";

/// The weight of the class default in an edge's mean (a station right
/// next to an edge has weight 1).
pub const PRIOR_WEIGHT: f64 = 0.1;

/// Weights below this end the propagation.
const MIN_WEIGHT: f64 = 0.03;

/// The weight kept per junction passed.
const JUNCTION_KEEP: f64 = 0.8;

/// An edge with the station's road number may be this many times the
/// tolerance away (wide motorways: a station's point may sit in the
/// median or beside the carriageways; the number makes the match sure).
const REF_REACH: f64 = 2.5;

/// Two candidates within this score are ambiguous (m).
const AMBIGUOUS_M: f64 = 8.0;

/// Flow factors are kept within this range.
pub const FLOW_RANGE: (f64, f64) = (0.05, 60.0);

/// One open source: its attribution as the manifest records it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CountSource {
    pub id: String,
    pub title: String,
    pub licence: String,
    pub notice: String,
    pub url: Option<String>,
    pub year: Option<u32>,
}

impl CountSource {
    /// The manifest's entry (spec 4.7; LICENSE-DATA.md and the credits).
    pub fn attribution(&self) -> Attribution {
        Attribution {
            source: format!("Traffic counts: {}", self.title),
            licence: self.licence.clone(),
            notice: format!(
                "{} The road traffic's demand per road (section TRFC, flow) was derived from \
                 these counts (matched to the roads and spread along them, i.e. modified).",
                self.notice
            ),
            url: self.url.clone(),
        }
    }
}

/// A counting station.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// The road number as the source writes it ("TF-1", "B 500", "A 13").
    pub reference: Option<String>,
    /// Annual average daily traffic, both directions (vehicles a day).
    pub aadt: f64,
    /// Per direction (1, 2), where known.
    pub dir: Option<[f64; 2]>,
    /// Compass bearing of direction 1 (degrees), where known.
    pub dir1_bearing_deg: Option<f64>,
}

/// Reads a counts file.
pub fn parse_counts(text: &str) -> Result<(CountSource, Vec<Station>), String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(COUNTS_MAGIC) {
        return Err(format!(
            "not an OCW counts file (first line {COUNTS_MAGIC:?})"
        ));
    }
    let mut src = CountSource::default();
    let mut header: Option<Vec<String>> = None;
    let mut stations = Vec::new();
    for (n, line) in lines.enumerate() {
        let line = line.trim_end_matches('\r');
        if let Some(meta) = line.strip_prefix('#') {
            if let Some((k, v)) = meta.split_once(':') {
                let v = v.trim().to_string();
                match k.trim() {
                    "source" => src.id = v,
                    "title" => src.title = v,
                    "licence" => src.licence = v,
                    "notice" => src.notice = v,
                    "url" => src.url = (!v.is_empty()).then_some(v),
                    "year" => src.year = v.parse().ok(),
                    _ => {}
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let cells = split_csv(line);
        let Some(h) = &header else {
            header = Some(cells);
            continue;
        };
        let get = |k: &str| {
            h.iter()
                .position(|c| c == k)
                .and_then(|i| cells.get(i))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
        };
        let num = |k: &str| get(k).and_then(|s| s.parse::<f64>().ok());
        let at = || format!("line {}", n + 2);
        let aadt = num("aadt").ok_or_else(|| format!("{}: no aadt", at()))?;
        let dir = match (num("aadt_dir1"), num("aadt_dir2")) {
            (Some(a), Some(b)) if a + b > 0.0 => Some([a, b]),
            _ => None,
        };
        stations.push(Station {
            id: get("id").unwrap_or_default().to_string(),
            name: get("name").unwrap_or_default().to_string(),
            lat: num("lat").ok_or_else(|| format!("{}: no lat", at()))?,
            lon: num("lon").ok_or_else(|| format!("{}: no lon", at()))?,
            reference: get("ref").map(str::to_string),
            aadt,
            dir,
            dir1_bearing_deg: num("dir1_bearing_deg"),
        });
    }
    if src.id.is_empty() || src.licence.is_empty() || src.notice.is_empty() {
        return Err("the counts file needs source, licence and notice lines".into());
    }
    Ok((src, stations))
}

/// One CSV line (double quotes around cells with commas, `""` inside).
fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// A road number for comparing: upper case without spaces, hyphens and
/// dots ("TF-1" → "TF1", "B 500" → "B500", "A 13" → "A13").
pub fn norm_ref(r: &str) -> String {
    r.chars()
        .filter(|c| !matches!(c, ' ' | '-' | '.' | '_'))
        .flat_map(char::to_uppercase)
        .collect()
}

/// The OSM `ref` and `name` of the road ways, by way id (for the edges'
/// `osm_way`).
#[derive(Debug, Clone, Default)]
pub struct WayNames {
    pub by_way: HashMap<i64, (Vec<String>, Option<String>)>,
}

impl WayNames {
    /// Adds a way's tags (`ref` may list several numbers with `;`).
    pub fn insert(&mut self, way: i64, reference: Option<&str>, name: Option<&str>) {
        let refs = reference
            .map(|r| {
                r.split(';')
                    .map(norm_ref)
                    .filter(|r| !r.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        self.by_way.insert(way, (refs, name.map(str::to_string)));
    }

    /// One pass over the ways of PBF extracts (`highway=*` with a `ref`
    /// or a `name`).
    pub fn read_pbfs(paths: &[std::path::PathBuf]) -> Result<Self, String> {
        let mut out = Self::default();
        for p in paths {
            let found = crate::osm::par_elements(p, |el, v| {
                if let osmpbf::Element::Way(w) = el {
                    let (mut hw, mut r, mut n) = (false, None, None);
                    for (k, val) in w.tags() {
                        match k {
                            "highway" => hw = true,
                            "ref" => r = Some(val.to_string()),
                            "name" => n = Some(val.to_string()),
                            _ => {}
                        }
                    }
                    if hw && (r.is_some() || n.is_some()) {
                        v.push((w.id(), r, n));
                    }
                }
            })
            .map_err(|e| format!("{}: {e}", p.display()))?;
            for (id, r, n) in found {
                out.insert(id, r.as_deref(), n.as_deref());
            }
        }
        Ok(out)
    }

    fn refs(&self, way: i64) -> &[String] {
        self.by_way.get(&way).map_or(&[], |w| &w.0)
    }
}

/// Matching and propagation settings.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// Stations farther than this from every motor edge stay unmatched (m).
    pub tolerance_m: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self { tolerance_m: 40.0 }
    }
}

/// A station on its edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    /// Index into the stations given to [`match_stations`].
    pub station: usize,
    /// Edge index in the graph.
    pub edge: usize,
    /// Distance from the station to the edge (m).
    pub dist_m: f64,
    /// Position along the edge from its `from` (m).
    pub along_m: f64,
    /// The edge's daily volume (both directions on a two-way edge, its
    /// direction on a one-way edge).
    pub volume: f64,
    /// The other carriageway of a dual carriageway, with its volume.
    pub partner: Option<(usize, f64)>,
}

/// Something the matching reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Issue {
    /// No motor edge within the tolerance (the nearest one's distance and
    /// road: its numbers, else its name, else its class).
    Unmatched {
        station: usize,
        nearest: Option<(f64, String)>,
    },
    /// Edges near but all with another road number.
    RefMismatch {
        station: usize,
        wanted: String,
        found: Vec<String>,
        dist_m: f64,
    },
    /// Another road scores within [`AMBIGUOUS_M`] of the best (used).
    Ambiguous { station: usize, edges: [usize; 2] },
    /// Several stations on one edge: their mean is used.
    SharedEdge { edge: usize, stations: Vec<usize> },
}

/// The result of [`match_stations`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Matching {
    pub anchors: Vec<Anchor>,
    pub issues: Vec<Issue>,
    /// Stations outside the package (not looked at further).
    pub outside: Vec<usize>,
}

/// A point's distance to a polyline: (distance, along from the start,
/// the segment's unit direction).
fn to_polyline(p: [f64; 2], pts: &[[f64; 2]]) -> (f64, f64, [f64; 2]) {
    let mut best = (f64::MAX, 0.0, [1.0, 0.0]);
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dz * dz).sqrt();
        if len <= 0.0 {
            continue;
        }
        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / (len * len)).clamp(0.0, 1.0);
        let (qx, qz) = (a[0] + t * dx, a[1] + t * dz);
        let d = ((p[0] - qx).powi(2) + (p[1] - qz).powi(2)).sqrt();
        if d < best.0 {
            best = (d, acc + t * len, [dx / len, dz / len]);
        }
        acc += len;
    }
    best
}

fn edge_length(pts: &[[f64; 2]]) -> f64 {
    pts.windows(2)
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum()
}

/// A grid of the edges' boxes for the near-edge queries.
struct EdgeGrid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl EdgeGrid {
    fn new(graph: &RoadGraph, keep: &[bool], cell: f64) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, e) in graph.edges.iter().enumerate() {
            if !keep[i] || e.points.is_empty() {
                continue;
            }
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in &e.points {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
            let c = |v: f64| (v / cell).floor() as i64;
            for x in c(lo[0])..=c(hi[0]) {
                for z in c(lo[1])..=c(hi[1]) {
                    cells.entry((x, z)).or_default().push(i);
                }
            }
        }
        Self { cell, cells }
    }

    /// Edge indices whose box cells are within `r` of `p` (sorted, unique).
    fn near(&self, p: [f64; 2], r: f64) -> Vec<usize> {
        let c = |v: f64| (v / self.cell).floor() as i64;
        let mut out = Vec::new();
        for x in c(p[0] - r)..=c(p[0] + r) {
            for z in c(p[1] - r)..=c(p[1] + r) {
                if let Some(v) = self.cells.get(&(x, z)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Whether the edge carries motor traffic (`TRFC`).
fn motor_edges(graph: &RoadGraph, rules: &TrafficData) -> Vec<bool> {
    graph
        .edges
        .iter()
        .map(|e| rules.edges.get(&e.id).is_some_and(|r| r.has_traffic()))
        .collect()
}

/// The direction motor vehicles drive on a one-way edge (None: two-way).
fn oneway_sign(rules: &TrafficData, graph: &RoadGraph, e: usize) -> Option<f64> {
    match rules.edges.get(&graph.edges[e].id).map(|r| r.oneway) {
        Some(Oneway::Forward) => Some(1.0),
        Some(Oneway::Backward) => Some(-1.0),
        _ => None,
    }
}

fn class_penalty(c: RoadClass) -> f64 {
    match c {
        RoadClass::Service => 40.0,
        RoadClass::Track => 60.0,
        RoadClass::Residential => 15.0,
        RoadClass::Unclassified => 5.0,
        _ => 0.0,
    }
}

/// Lower case letters and digits only.
fn simple(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The road of an edge for "the same road": its numbers, else its name.
#[derive(Debug, Clone, PartialEq)]
enum RoadKey {
    Refs(Vec<String>),
    Name(String),
    None,
}

fn road_key(graph: &RoadGraph, names: &WayNames, e: usize) -> RoadKey {
    let edge = &graph.edges[e];
    let refs = names.refs(edge.osm_way);
    if !refs.is_empty() {
        RoadKey::Refs(refs.to_vec())
    } else if let Some(n) = edge.name.as_deref().filter(|n| !n.is_empty()) {
        RoadKey::Name(n.to_string())
    } else {
        RoadKey::None
    }
}

fn same_road(a: &RoadKey, b: &RoadKey) -> bool {
    match (a, b) {
        (RoadKey::Refs(x), RoadKey::Refs(y)) => x.iter().any(|r| y.contains(r)),
        (RoadKey::Name(x), RoadKey::Name(y)) => x == y,
        _ => false,
    }
}

/// Whether an edge's refs fit a station's number (a Swiss main road
/// "H 13" is "13" in OSM).
fn ref_fits(want: &str, refs: &[String]) -> bool {
    refs.iter()
        .any(|r| r == want || want.strip_prefix('H').is_some_and(|w| w == r))
}

#[derive(Debug, Clone)]
struct Candidate {
    edge: usize,
    dist: f64,
    /// The distance it may be at most (longer with the station's number).
    limit: f64,
    along: f64,
    dir: [f64; 2],
    score: f64,
}

/// Matches stations to edges (see the module docs). `inside` says
/// whether a local point is in the package (stations outside are only
/// counted).
pub fn match_stations(
    graph: &RoadGraph,
    rules: &TrafficData,
    names: &WayNames,
    proj: &LocalProjection,
    stations: &[Station],
    params: &Params,
    inside: impl Fn([f64; 2]) -> bool,
) -> Matching {
    let motor = motor_edges(graph, rules);
    let grid = EdgeGrid::new(graph, &motor, 250.0);
    let mut m = Matching::default();
    let reach = params.tolerance_m * REF_REACH + 40.0;
    for (si, st) in stations.iter().enumerate() {
        let p = proj.project(st.lat, st.lon);
        if !inside(p) {
            m.outside.push(si);
            continue;
        }
        let want = st.reference.as_deref().map(norm_ref);
        let station_name = simple(&st.name);
        let mut cands = Vec::new();
        let mut mismatched: Vec<(f64, String)> = Vec::new();
        let mut nearest: Option<(f64, String)> = None;
        for e in grid.near(p, reach) {
            let edge = &graph.edges[e];
            let (dist, along, dir) = to_polyline(p, &edge.points);
            if nearest.as_ref().is_none_or(|n| dist < n.0) {
                let refs = names.refs(edge.osm_way);
                let road = if refs.is_empty() {
                    edge.name
                        .clone()
                        .unwrap_or_else(|| format!("{:?}", edge.class))
                } else {
                    refs.join(";")
                };
                nearest = Some((dist, road));
            }
            if dist > reach {
                continue;
            }
            let refs = names.refs(edge.osm_way);
            let mut score = dist + class_penalty(edge.class);
            let mut limit = params.tolerance_m;
            if let Some(w) = &want {
                if refs.is_empty() {
                    score += 30.0;
                } else if ref_fits(w, refs) {
                    limit *= REF_REACH;
                } else {
                    if dist <= params.tolerance_m {
                        mismatched.push((dist, refs.join(";")));
                    }
                    continue;
                }
            } else if let Some(n) = edge.name.as_deref().map(simple)
                && n.len() >= 4
                && station_name.contains(&n)
            {
                score -= 10.0;
            }
            cands.push(Candidate {
                edge: e,
                dist,
                limit,
                along,
                dir,
                score,
            });
        }
        cands.sort_by(|a, b| a.score.total_cmp(&b.score).then(a.edge.cmp(&b.edge)));
        let Some(best) = cands.iter().find(|c| c.dist <= c.limit).cloned() else {
            if let (Some(w), Some((d, _))) = (&want, mismatched.first()) {
                let mut found: Vec<String> = mismatched.iter().map(|x| x.1.clone()).collect();
                found.sort();
                found.dedup();
                m.issues.push(Issue::RefMismatch {
                    station: si,
                    wanted: w.clone(),
                    found,
                    dist_m: *d,
                });
            } else {
                m.issues.push(Issue::Unmatched {
                    station: si,
                    nearest,
                });
            }
            continue;
        };
        let key = road_key(graph, names, best.edge);
        let sign = oneway_sign(rules, graph, best.edge);
        // The other carriageway: one-way, the same road, driven the other way.
        let partner = sign.and_then(|s| {
            let drive = [best.dir[0] * s, best.dir[1] * s];
            cands
                .iter()
                .filter(|c| c.edge != best.edge)
                .filter(|c| {
                    let k = road_key(graph, names, c.edge);
                    k == key || same_road(&k, &key)
                })
                .filter_map(|c| {
                    let cs = oneway_sign(rules, graph, c.edge)?;
                    let d2 = [c.dir[0] * cs, c.dir[1] * cs];
                    (drive[0] * d2[0] + drive[1] * d2[1] < -0.3).then_some(c.clone())
                })
                .min_by(|a, b| a.dist.total_cmp(&b.dist).then(a.edge.cmp(&b.edge)))
        });
        // Another road almost as good: ambiguous (the best is used).
        if let Some(other) = cands.iter().find(|c| {
            c.edge != best.edge
                && c.dist <= c.limit
                && partner.as_ref().is_none_or(|p| p.edge != c.edge)
                && !same_road(&road_key(graph, names, c.edge), &key)
                && !(key == RoadKey::None && road_key(graph, names, c.edge) == RoadKey::None)
        }) && other.score - best.score < AMBIGUOUS_M
        {
            m.issues.push(Issue::Ambiguous {
                station: si,
                edges: [best.edge, other.edge],
            });
        }
        // The volumes: the direction where the edge is one-way.
        let (volume, partner) = match sign {
            None => (st.aadt, None),
            Some(s) => {
                let drive = [best.dir[0] * s, best.dir[1] * s];
                let (mine, theirs) = match (st.dir, st.dir1_bearing_deg) {
                    (Some([a, b]), Some(brg)) => {
                        let r = brg.to_radians();
                        let d1 = [r.sin(), -r.cos()];
                        if drive[0] * d1[0] + drive[1] * d1[1] >= 0.0 {
                            (a, b)
                        } else {
                            (b, a)
                        }
                    }
                    // A one-way street counted in one direction only.
                    (Some([a, b]), None) if partner.is_none() && (a == 0.0 || b == 0.0) => {
                        (a + b, 0.0)
                    }
                    _ => (st.aadt / 2.0, st.aadt / 2.0),
                };
                (mine, partner.map(|p| (p.edge, theirs)))
            }
        };
        m.anchors.push(Anchor {
            station: si,
            edge: best.edge,
            dist_m: best.dist,
            along_m: best.along,
            volume,
            partner,
        });
    }
    // Several stations on one edge: reported (the mean is used).
    let mut by_edge: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for a in &m.anchors {
        by_edge.entry(a.edge).or_default().push(a.station);
        if let Some((p, _)) = a.partner {
            by_edge.entry(p).or_default().push(a.station);
        }
    }
    for (edge, stations) in by_edge {
        if stations.len() > 1 {
            m.issues.push(Issue::SharedEdge { edge, stations });
        }
    }
    m
}

/// The decay length of a counted road by its class (m).
fn decay_length(c: RoadClass) -> f64 {
    match c {
        RoadClass::Trunk => 10_000.0,
        RoadClass::Primary => 6_000.0,
        RoadClass::Secondary => 4_000.0,
        RoadClass::Tertiary => 2_500.0,
        _ => 1_200.0,
    }
}

/// A Dijkstra entry: the weight's log (larger first), the edge, the
/// distance to the edge's middle and the junctions passed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reach {
    weight: f64,
    edge: usize,
    /// The vertex the walk entered the edge by (usize::MAX at the anchor).
    via: usize,
    dist: f64,
    junctions: u32,
}

impl Eq for Reach {}

impl Ord for Reach {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.weight
            .total_cmp(&o.weight)
            .then(o.edge.cmp(&self.edge))
            .then(o.via.cmp(&self.via))
    }
}

impl PartialOrd for Reach {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// The per-edge flow factors from the matched stations: `Some(factor)`
/// for the edges the counts reach, `None` elsewhere (keep 1).
/// `unit_volume(e)` is the daily volume edge `e` carries with factor 1
/// (the sim's class default × the annual day factor, in the edge's
/// directions: both on a two-way edge, its own on a one-way edge).
pub fn flow_factors(
    graph: &RoadGraph,
    rules: &TrafficData,
    names: &WayNames,
    matching: &Matching,
    unit_volume: impl Fn(usize) -> f64,
) -> Vec<Option<f32>> {
    let n = graph.edges.len();
    let motor = motor_edges(graph, rules);
    let adj = graph.adjacency();
    let motor_degree: Vec<usize> = adj
        .iter()
        .map(|v| v.iter().filter(|&&e| motor[e]).count())
        .collect();
    let lengths: Vec<f64> = graph.edges.iter().map(|e| edge_length(&e.points)).collect();
    let share = |e: usize| {
        if oneway_sign(rules, graph, e).is_some() {
            0.5
        } else {
            1.0
        }
    };
    // Counted edges: their counts (the mean of several).
    let mut counted: BTreeMap<usize, (f64, u32)> = BTreeMap::new();
    for a in &matching.anchors {
        let c = counted.entry(a.edge).or_default();
        c.0 += a.volume;
        c.1 += 1;
        if let Some((p, v)) = a.partner {
            let c = counted.entry(p).or_default();
            c.0 += v;
            c.1 += 1;
        }
    }
    // Spread: per edge the (weight, cross-section volume) of each station.
    let mut reached: Vec<Vec<(f64, f64)>> = vec![Vec::new(); n];
    for a in &matching.anchors {
        let cross = if oneway_sign(rules, graph, a.edge).is_some() && a.partner.is_some() {
            a.volume + a.partner.map_or(0.0, |p| p.1)
        } else {
            a.volume / share(a.edge)
        };
        let key = road_key(graph, names, a.edge);
        let len_scale = decay_length(graph.edges[a.edge].class);
        let mut best: HashMap<usize, f64> = HashMap::new();
        let mut heap = BinaryHeap::new();
        for edge in std::iter::once(a.edge).chain(a.partner.map(|p| p.0)) {
            heap.push(Reach {
                weight: 1.0,
                edge,
                via: usize::MAX,
                dist: 0.0,
                junctions: 0,
            });
        }
        while let Some(r) = heap.pop() {
            if best.get(&r.edge).is_some_and(|&w| w >= r.weight) {
                continue;
            }
            best.insert(r.edge, r.weight);
            let e = &graph.edges[r.edge];
            for v in [e.from.0 as usize, e.to.0 as usize] {
                if v == r.via {
                    continue;
                }
                // From the station (on its edge) or the middle to the vertex.
                let to_v = if r.via == usize::MAX && r.edge == a.edge {
                    if v == e.from.0 as usize {
                        a.along_m
                    } else {
                        (lengths[r.edge] - a.along_m).max(0.0)
                    }
                } else {
                    lengths[r.edge] / 2.0
                };
                let junction = u32::from(motor_degree[v] >= 3);
                for &x in &adj[v] {
                    if x == r.edge || !motor[x] {
                        continue;
                    }
                    let fits = match &key {
                        RoadKey::None => motor_degree[v] == 2,
                        k => same_road(k, &road_key(graph, names, x)),
                    };
                    if !fits {
                        continue;
                    }
                    let dist = r.dist + to_v + lengths[x] / 2.0;
                    let junctions = r.junctions + junction;
                    let weight = (-dist / len_scale).exp() * JUNCTION_KEEP.powi(junctions as i32);
                    if weight < MIN_WEIGHT || best.get(&x).is_some_and(|&w| w >= weight) {
                        continue;
                    }
                    heap.push(Reach {
                        weight,
                        edge: x,
                        via: v,
                        dist,
                        junctions,
                    });
                }
            }
        }
        let mut got: Vec<(usize, f64)> = best.into_iter().collect();
        got.sort_unstable_by_key(|g| g.0);
        for (e, w) in got {
            reached[e].push((w, cross));
        }
    }
    (0..n)
        .map(|e| {
            let unit = unit_volume(e);
            if !motor[e] || unit <= 0.0 {
                return None;
            }
            let volume = if let Some(&(sum, k)) = counted.get(&e) {
                sum / f64::from(k)
            } else if reached[e].is_empty() {
                return None;
            } else {
                let s = share(e);
                let (wsum, vsum) = reached[e]
                    .iter()
                    .fold((0.0, 0.0), |(w, v), &(wi, ci)| (w + wi, v + wi * ci * s));
                (vsum + PRIOR_WEIGHT * unit) / (wsum + PRIOR_WEIGHT)
            };
            Some((volume / unit).clamp(FLOW_RANGE.0, FLOW_RANGE.1) as f32)
        })
        .collect()
}

/// Sets the factors into the traffic data: every edge back to 1 first
/// (a second run gives the same), then the counted ones. Returns the
/// number of edges with a factor.
pub fn apply_flows(graph: &RoadGraph, rules: &mut TrafficData, flows: &[Option<f32>]) -> usize {
    for r in rules.edges.values_mut() {
        r.flow = 1.0;
    }
    let mut n = 0;
    for (e, f) in graph.edges.iter().zip(flows) {
        if let (Some(f), Some(r)) = (f, rules.edges.get_mut(&e.id)) {
            r.flow = *f;
            n += 1;
        }
    }
    n
}

/// Reads a counts file from disk.
pub fn read_counts(path: &Path) -> Result<(CountSource, Vec<Station>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_counts(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The diverging colour of a factor: blue below 1, grey at 1, red above
/// (log scale, saturated at ¼ and 4).
pub fn flow_colour(f: f64) -> String {
    // The validated diverging pair (blue #2a78d6, red #e34948) and the
    // neutral midpoint #c9c8c4 (darker than the surface so roads show).
    let t = (f.max(1e-3).log2() / 2.0).clamp(-1.0, 1.0);
    let mid = [0xc9, 0xc8, 0xc4];
    let end = if t < 0.0 {
        [0x2a, 0x78, 0xd6]
    } else {
        [0xe3, 0x49, 0x48]
    };
    let a = t.abs().powf(0.7);
    let c: Vec<u8> = (0..3)
        .map(|i| (f64::from(mid[i]) + (f64::from(end[i]) - f64::from(mid[i])) * a).round() as u8)
        .collect();
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// An SVG map of the motor roads coloured by their flow factor, with the
/// matched stations (filled) and the unmatched ones inside (rings).
pub fn flow_svg(
    title: &str,
    graph: &RoadGraph,
    rules: &TrafficData,
    proj: &LocalProjection,
    stations: &[Station],
    matching: &Matching,
) -> String {
    let motor = motor_edges(graph, rules);
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (e, m) in graph.edges.iter().zip(&motor) {
        if *m {
            for p in &e.points {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
    }
    if lo[0] > hi[0] {
        lo = [0.0, 0.0];
        hi = [1.0, 1.0];
    }
    let w_px = 1400.0;
    let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
    let s = (w_px - 80.0) / span;
    let h_px = (hi[1] - lo[1]) * s + 230.0;
    let w_px = (hi[0] - lo[0]) * s + 80.0;
    let w_px = w_px.max(720.0);
    let tx = |p: [f64; 2]| ((p[0] - lo[0]) * s + 40.0, (p[1] - lo[1]) * s + 140.0);
    let mut o = String::new();
    let _ = writeln!(
        o,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w_px:.0}" height="{h_px:.0}" viewBox="0 0 {w_px:.0} {h_px:.0}" font-family="sans-serif">"##
    );
    let _ = writeln!(o, r##"<rect width="100%" height="100%" fill="#fcfcfb"/>"##);
    let _ = writeln!(
        o,
        r##"<text x="40" y="38" font-size="22" fill="#1a1a19">{}</text>"##,
        xml(title)
    );
    let _ = writeln!(
        o,
        r##"<text x="40" y="62" font-size="14" fill="#5f5e5a">Flow factor per road: counted daily traffic ÷ the class default</text><text x="40" y="82" font-size="14" fill="#5f5e5a">Filled dots: matched stations; rings: unmatched</text>"##
    );
    // Legend: the factor bins.
    let bins = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 4.0];
    for (i, f) in bins.iter().enumerate() {
        let x = 40.0 + i as f64 * 70.0;
        let _ = writeln!(
            o,
            r##"<rect x="{x}" y="94" width="60" height="8" rx="2" fill="{}"/><text x="{}" y="118" font-size="12" fill="#5f5e5a" text-anchor="middle">{f}</text>"##,
            flow_colour(*f),
            x + 30.0
        );
    }
    // Roads: those at 1 first, the counted ones on top.
    let mut order: Vec<usize> = (0..graph.edges.len()).filter(|&e| motor[e]).collect();
    let flow = |e: usize| f64::from(rules.edges.get(&graph.edges[e].id).map_or(1.0, |r| r.flow));
    order.sort_by(|&a, &b| {
        (flow(a) - 1.0)
            .abs()
            .total_cmp(&(flow(b) - 1.0).abs())
            .then(a.cmp(&b))
    });
    for e in order {
        let edge = &graph.edges[e];
        let width = match edge.class {
            RoadClass::Trunk => 3.2,
            RoadClass::Primary => 2.6,
            RoadClass::Secondary => 2.2,
            RoadClass::Tertiary => 1.8,
            RoadClass::Unclassified | RoadClass::Residential => 1.0,
            _ => 0.7,
        };
        let mut d = String::new();
        for (i, p) in edge.points.iter().enumerate() {
            let (x, y) = tx(*p);
            let _ = write!(d, "{}{x:.1},{y:.1} ", if i == 0 { "M" } else { "L" });
        }
        let _ = writeln!(
            o,
            r##"<path d="{d}" fill="none" stroke="{}" stroke-width="{width}" stroke-linecap="round"/>"##,
            flow_colour(flow(e))
        );
    }
    let matched: std::collections::BTreeSet<usize> =
        matching.anchors.iter().map(|a| a.station).collect();
    for (i, st) in stations.iter().enumerate() {
        if matching.outside.contains(&i) {
            continue;
        }
        let (x, y) = tx(proj.project(st.lat, st.lon));
        if matched.contains(&i) {
            let _ = writeln!(
                o,
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="4.5" fill="#1a1a19" stroke="#fcfcfb" stroke-width="2"><title>{} {}: {:.0} a day</title></circle>"##,
                xml(&st.id),
                xml(&st.name),
                st.aadt
            );
        } else {
            let _ = writeln!(
                o,
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="4.5" fill="none" stroke="#1a1a19" stroke-width="1.5"><title>{} {} (unmatched)</title></circle>"##,
                xml(&st.id),
                xml(&st.name)
            );
        }
    }
    o.push_str("</svg>\n");
    o
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests;
