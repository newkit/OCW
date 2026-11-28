//! Tunnels in the terrain (spec 4.3 *Tunnels and bridges in the terrain*,
//! 4.8.1 the `tunnel.*` check): measuring how much of each tunnel tube
//! the terrain leaves in the open, shaping the terrain so the renderer's
//! tube is inside the hill, and the build check.
//!
//! The renderer draws a tube where the terrain covers a tunnel road; on
//! the Andorra package (2026-11-28) many tubes stood in the open. The
//! sections of each tunnel ([`ocw_world::tunnels`]) say what it is in the
//! terrain; [`shape`] then
//!
//! - raises the terrain over a bored stretch whose shell is not covered by
//!   [`COVER_M`] (a mound falling off into the slope at 1:1.5), never under
//!   another road,
//! - lays the terrain under the road of open stretches and galleries just
//!   below the road (cut and fill across the carriageway, as at every
//!   other road), so the road neither floats nor is buried and the portal
//!   is where the road enters the hill,
//!
//! and [`check`] reports what is still exposed (the build fails on it).

use std::collections::{BTreeMap, HashMap};

use ocw_world::graph::{Edge, RoadGraph};
use ocw_world::package::{TileKey, WorldPackage};
use ocw_world::tunnels::{
    self as tn, AxisSample, COVER_M, CROWN_M, LINING_M, Section, SectionKind,
};

use crate::spatial::SegmentIndex;

/// OSM `tunnel=*` value of each tunnel way (from the extract; `yes` when
/// unknown).
pub type TunnelTags = HashMap<i64, String>;

/// The `tunnel=*` values of the tunnel ways in `osm`.
pub fn tunnel_tags(osm: &crate::osm::OsmData) -> TunnelTags {
    osm.ways
        .iter()
        .filter_map(|w| {
            let t = w.tag("tunnel")?;
            (t != "no").then(|| (w.id, t.to_string()))
        })
        .collect()
}

/// What a `tunnel=*` value says about the structure: a passage under a
/// building (the building is the roof) or a culvert is open; an
/// `avalanche_protector` is a gallery over its whole length.
pub fn tag_kind(tag: Option<&str>) -> Option<SectionKind> {
    match tag {
        Some("building_passage" | "culvert" | "flooded") => Some(SectionKind::Open),
        // The hill side is decided from the terrain ([`classify`]).
        Some("avalanche_protector") => Some(SectionKind::Gallery { hill_right: true }),
        _ => None,
    }
}

/// The sections of `e` in `pkg`'s terrain: those the package stores
/// (section `TUNL`, written by [`shape`]) or, without them, classified
/// from the terrain with the OSM tag's say ([`classify`]).
pub fn edge_sections(
    pkg: &WorldPackage,
    e: &Edge,
    tag: Option<&str>,
) -> (Vec<AxisSample>, Vec<Section>) {
    let ground = |x: f64, z: f64| pkg.surface_height(x, z);
    let samples = tn::probe(e, &ground);
    if let Some(stored) = pkg.look.tunnels.as_ref().and_then(|t| t.edges.get(&e.id.0)) {
        return (samples, stored.clone());
    }
    let secs = classify(&samples, tag);
    (samples, secs)
}

/// The sections of a tunnel's samples with its OSM tag.
pub fn classify(samples: &[AxisSample], tag: Option<&str>) -> Vec<Section> {
    let (Some(a), Some(b)) = (samples.first(), samples.last()) else {
        return Vec::new();
    };
    match tag_kind(tag) {
        Some(SectionKind::Gallery { .. }) => {
            // The higher side is the hill.
            let (l, r) = samples
                .iter()
                .fold((0.0, 0.0), |(l, r), x| (l + x.side[0], r + x.side[1]));
            vec![Section {
                from: a.s,
                to: b.s,
                kind: SectionKind::Gallery { hill_right: r >= l },
            }]
        }
        Some(kind) => vec![Section {
            from: a.s,
            to: b.s,
            kind,
        }],
        None => tn::sections(samples),
    }
}

/// One tunnel's measure.
#[derive(Debug, Clone, PartialEq)]
pub struct TunnelMeasure {
    pub edge: usize,
    pub osm_way: i64,
    pub name: Option<String>,
    pub tag: String,
    pub length_m: f64,
    /// Where the terrain covers the road centre by 0.5 m (the renderer
    /// before 2026-11-28 drew its tube there), m.
    pub covered_m: f64,
    /// Of that, where the shell stuck out by more than
    /// [`tn::EXPOSED_TOL_M`] (8 m inside each end left out), m.
    pub old_exposed_m: f64,
    /// The worst exposure of the shell there (m) and where.
    pub worst_m: f64,
    pub worst_at: [f64; 2],
    /// The tunnel's direction there (x, z unit).
    pub worst_dir: [f64; 2],
    /// The least and the median cover over the shell along the tunnel (m).
    pub min_cover_m: f64,
    pub median_cover_m: f64,
    /// Where the terrain at the centre is below the road (m of road).
    pub floating_m: f64,
    /// The sections by kind (m): bored, gallery, open.
    pub bored_m: f64,
    pub gallery_m: f64,
    pub open_m: f64,
    /// Bored stretches still exposed ([`tn::exposed_runs`]): length (m)
    /// and the worst (m).
    pub exposed_m: f64,
    pub exposed_worst_m: f64,
    pub sections: Vec<Section>,
}

fn section_lengths(secs: &[Section]) -> (f64, f64, f64) {
    let mut out = (0.0, 0.0, 0.0);
    for s in secs {
        let l = s.to - s.from;
        match s.kind {
            SectionKind::Bored => out.0 += l,
            SectionKind::Gallery { .. } => out.1 += l,
            SectionKind::Open => out.2 += l,
        }
    }
    out
}

/// Measures every tunnel edge of `pkg`.
pub fn measure(pkg: &WorldPackage, tags: &TunnelTags) -> Vec<TunnelMeasure> {
    let Some(g) = &pkg.graph else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (ei, e) in g.edges.iter().enumerate().filter(|(_, e)| e.tunnel) {
        let tag = tags.get(&e.osm_way).map(String::as_str);
        let (samples, secs) = edge_sections(pkg, e, tag);
        if samples.is_empty() {
            continue;
        }
        let step = tn::STEP_M;
        // The old renderer's tube: centre covered by 0.5 m.
        let covered: Vec<bool> = samples.iter().map(|x| x.centre > 0.5).collect();
        let covered_m = covered.iter().filter(|c| **c).count() as f64 * step;
        let mut old_exposed_m = 0.0;
        let (mut worst_m, mut worst_at, mut worst_dir) = (0.0, samples[0].p, samples[0].dir);
        // Runs of covered samples, 8 m inside each end left out.
        let mut i = 0;
        while i < samples.len() {
            if !covered[i] {
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < samples.len() && covered[j + 1] {
                j += 1;
            }
            let (a, b) = (samples[i].s + 8.0, samples[j].s - 8.0);
            for x in &samples[i..=j] {
                if x.s < a || x.s > b {
                    continue;
                }
                let ex = -x.roof_cover();
                if ex > tn::EXPOSED_TOL_M {
                    old_exposed_m += step;
                    if ex > worst_m {
                        worst_m = ex;
                        worst_at = x.p;
                        worst_dir = x.dir;
                    }
                }
            }
            i = j + 1;
        }
        let mut covers: Vec<f64> = samples.iter().map(AxisSample::roof_cover).collect();
        covers.sort_by(f64::total_cmp);
        let floating_m = samples.iter().filter(|x| x.centre < -0.5).count() as f64 * step;
        let (bored_m, gallery_m, open_m) = section_lengths(&secs);
        let ex = tn::exposed_runs(&samples, &secs);
        out.push(TunnelMeasure {
            edge: ei,
            osm_way: e.osm_way,
            name: e.name.clone(),
            tag: tag.unwrap_or("yes").to_string(),
            length_m: e.length_m(),
            covered_m,
            old_exposed_m,
            worst_m,
            worst_at,
            worst_dir,
            min_cover_m: covers[0],
            median_cover_m: covers[covers.len() / 2],
            floating_m,
            bored_m,
            gallery_m,
            open_m,
            exposed_m: ex.iter().map(|(a, b, _)| b - a + step).sum(),
            exposed_worst_m: ex.iter().map(|r| r.2).fold(0.0, f64::max),
            sections: secs,
        });
    }
    out
}

/// Stretches of other roads where the terrain covers the road centre by
/// 0.5 m over 10 m or more (the renderer's tunnel finder sees a tunnel
/// there): (edge, from, to, the most cover m).
pub fn buried_roads(pkg: &WorldPackage) -> Vec<(usize, f64, f64, f64)> {
    let Some(g) = &pkg.graph else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (ei, e) in g.edges.iter().enumerate() {
        if e.tunnel || e.bridge || e.profile.is_empty() {
            continue;
        }
        let len = e.length_m();
        let n = (len / tn::STEP_M).ceil() as usize;
        let mut run: Option<(f64, f64, f64)> = None;
        let mut flush = |run: &mut Option<(f64, f64, f64)>| {
            if let Some((a, b, w)) = run.take()
                && b - a >= 10.0
            {
                out.push((ei, a, b, w));
            }
        };
        for i in 0..=n {
            let s = (i as f64 * tn::STEP_M).min(len);
            let (p, _) = tn::point_dir(&e.points, s);
            let (Some(y), Some(h)) = (e.height_at(s), pkg.surface_height(p[0], p[1])) else {
                continue;
            };
            if h > y + 0.5 {
                run = Some(match run {
                    Some((a, _, w)) => (a, s, w.max(h - y)),
                    None => (s, s, h - y),
                });
            } else {
                flush(&mut run);
            }
        }
        flush(&mut run);
    }
    out
}

/// What [`shape`] changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapeStats {
    pub tunnels: usize,
    /// Terrain vertices raised (mounds) and the most (m).
    pub raised: usize,
    pub max_raise_m: f64,
    /// Vertices laid under open roads and galleries, and the most moved.
    pub laid: usize,
    pub max_lay_m: f64,
    /// Mound vertices left as they were because another road runs there
    /// (or the mound would rise more than [`MOUND_RAISE_MAX_M`]).
    pub kept_for_roads: usize,
    /// Vertices under an open road or a gallery more than [`FILL_MAX_M`]
    /// below it, left as they were.
    pub left_floating: usize,
    /// Bored stretches the mounds could not cover, made open road in a
    /// cutting (m; under another road: an underpass).
    pub cut_open_m: f64,
}

/// The terrain below the road surface where the road is in the open
/// (as the other roads' cut, spec 4.3).
const UNDER_ROAD_M: f64 = 0.3;
/// The terrain is filled up under an open road or a gallery by at most
/// this (m).
pub const FILL_MAX_M: f64 = 4.0;
/// A mound raises a vertex by at most this (m).
pub const MOUND_RAISE_MAX_M: f64 = 9.0;

type Targets = BTreeMap<(i64, i64), f64>;

/// The grid vertices across the road of sample `x` (a band of `band` m
/// each side, the nearest sample along the road), set to just below the
/// road in `lay`.
fn lay_band(x: &AxisSample, band: f64, sp: f64, lay: &mut Targets) {
    for v in vertices_near(x.p, band + tn::STEP_M, sp) {
        let q = [v.0 as f64 * sp, v.1 as f64 * sp];
        let (dx, dz) = (q[0] - x.p[0], q[1] - x.p[1]);
        let along = dx * x.dir[0] + dz * x.dir[1];
        let lateral = -dx * x.dir[1] + dz * x.dir[0];
        if along.abs() > tn::STEP_M * 0.75 || lateral.abs() > band {
            continue;
        }
        let h = x.road_y - UNDER_ROAD_M;
        let t = lay.entry(v).or_insert(f64::INFINITY);
        *t = t.min(h);
    }
}

/// The half width of the band laid under an open road / a gallery.
fn band_of(kind: SectionKind, x: &AxisSample, width: f64, sp: f64) -> f64 {
    let half = match kind {
        SectionKind::Open => width / 2.0 + 0.5,
        _ => x.half,
    };
    half + sp * 0.5
}

/// Applies the raise (mounds: up only, never near another road) and lay
/// (cut any, fill at most [`FILL_MAX_M`]) targets to the tiles; shared
/// border vertices get the same value.
fn apply(
    pkg: &mut WorldPackage,
    g: &RoadGraph,
    roads: &SegmentIndex,
    raise: &Targets,
    lay: &Targets,
    st: &mut ShapeStats,
) {
    let size = f64::from(pkg.tile_size_m);
    let n = pkg.samples as i64;
    let sp = pkg.spacing_m();
    let keys: Vec<TileKey> = pkg.tiles.keys().copied().collect();
    let mut kept: std::collections::BTreeSet<(i64, i64)> = Default::default();
    let mut floating: std::collections::BTreeSet<(i64, i64)> = Default::default();
    let mut counted: std::collections::BTreeSet<(i64, i64)> = Default::default();
    for key in keys {
        let o = key.origin(size);
        let (gx0, gz0) = ((o[0] / sp).round() as i64, (o[1] / sp).round() as i64);
        let tile = pkg.tiles.get_mut(&key).expect("key");
        for k in 0..n {
            for i in 0..n {
                let v = (gx0 + i, gz0 + k);
                let (r, l) = (raise.get(&v), lay.get(&v));
                if r.is_none() && l.is_none() {
                    continue;
                }
                let idx = (k * n + i) as usize;
                let cur = f64::from(tile.heights[idx]);
                let mut h = cur;
                if let Some(&t) = r
                    && t > h
                {
                    let q = [v.0 as f64 * sp, v.1 as f64 * sp];
                    // Never under another road (it stays on its own
                    // terrain; a road over the tunnel is conformed).
                    let busy = roads
                        .near(g, q, 12.0)
                        .iter()
                        .any(|hit| hit.dist <= g.edges[hit.edge].width_m / 2.0 + 3.0);
                    if busy || t - h > MOUND_RAISE_MAX_M {
                        kept.insert(v);
                    } else {
                        h = t;
                    }
                }
                if let Some(&t) = l {
                    // Cut down any amount; fill up at most FILL_MAX_M (a
                    // road higher above the valley is left floating and
                    // counted: a viaduct, not an embankment).
                    if t < h || t - h <= FILL_MAX_M {
                        h = t;
                    } else {
                        floating.insert(v);
                    }
                }
                if (h - cur).abs() > 1e-3 {
                    tile.heights[idx] = h as f32;
                    if counted.insert(v) {
                        if l.is_some() {
                            st.laid += 1;
                            st.max_lay_m = st.max_lay_m.max((h - cur).abs());
                        } else {
                            st.raised += 1;
                            st.max_raise_m = st.max_raise_m.max(h - cur);
                        }
                    }
                }
            }
        }
    }
    st.kept_for_roads += kept.len();
    st.left_floating += floating.len();
}

/// `secs` with `[a, b]` of its bored sections made open; bored pieces
/// left shorter than [`tn::MIN_SECTION_M`] are open too.
pub fn carve_open(secs: &[Section], a: f64, b: f64) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for s in secs {
        if s.kind != SectionKind::Bored || s.to <= a || s.from >= b {
            out.push(*s);
            continue;
        }
        if a > s.from {
            out.push(Section {
                from: s.from,
                to: a,
                kind: SectionKind::Bored,
            });
        }
        out.push(Section {
            from: a.max(s.from),
            to: b.min(s.to),
            kind: SectionKind::Open,
        });
        if b < s.to {
            out.push(Section {
                from: b,
                to: s.to,
                kind: SectionKind::Bored,
            });
        }
    }
    for s in &mut out {
        if s.kind == SectionKind::Bored && s.to - s.from < tn::MIN_SECTION_M {
            s.kind = SectionKind::Open;
        }
    }
    let mut merged: Vec<Section> = Vec::new();
    for s in out {
        match merged.last_mut() {
            Some(m) if m.kind == s.kind => m.to = s.to,
            _ => merged.push(s),
        }
    }
    merged
}

/// Shapes `pkg`'s terrain round its tunnels (see the module docs) and
/// stores their sections (section `TUNL`). Deterministic: the targets are
/// collected per global grid vertex and applied in key order.
pub fn shape(pkg: &mut WorldPackage, tags: &TunnelTags) -> ShapeStats {
    let mut st = ShapeStats::default();
    let Some(g) = pkg.graph.clone() else {
        return st;
    };
    let g: &RoadGraph = &g;
    let sp = pkg.spacing_m();
    let roads = SegmentIndex::new(g, 20.0, |e| !g.edges[e].tunnel);
    // Global vertex (gx, gz) → raise target (max) / lay target (min).
    let mut raise = Targets::new();
    let mut lay = Targets::new();
    let mut stored = ocw_world::tunnels::TunnelSections::default();
    for e in g.edges.iter().filter(|e| e.tunnel) {
        let tag = tags.get(&e.osm_way).map(String::as_str);
        let ground = |x: f64, z: f64| pkg.surface_height(x, z);
        let samples = tn::probe(e, &ground);
        let secs = classify(&samples, tag);
        if samples.is_empty() {
            continue;
        }
        st.tunnels += 1;
        for x in &samples {
            let kind = section_kind_at(&secs, x.s);
            match kind {
                SectionKind::Bored => {
                    if x.roof_cover() >= COVER_M - LINING_M {
                        continue;
                    }
                    let top = x.road_y + CROWN_M + LINING_M + COVER_M;
                    let reach = x.half + (top - x.road_y) * 1.5;
                    for v in vertices_near(x.p, reach, sp) {
                        let q = [v.0 as f64 * sp, v.1 as f64 * sp];
                        let d = (q[0] - x.p[0]).hypot(q[1] - x.p[1]);
                        let h = top - (d - x.half).max(0.0) / 1.5;
                        if h <= x.road_y {
                            continue;
                        }
                        let t = raise.entry(v).or_insert(f64::NEG_INFINITY);
                        *t = t.max(h);
                    }
                }
                SectionKind::Gallery { .. } | SectionKind::Open => {
                    lay_band(x, band_of(kind, x, e.width_m, sp), sp, &mut lay);
                }
            }
        }
        stored.edges.insert(e.id.0, secs);
    }
    apply(pkg, g, &roads, &raise, &lay, &mut st);
    // What the mounds could not cover (another road over the tunnel, a
    // shell far out of a steep slope) becomes open road in a cutting –
    // under another road an underpass, that road its roof.
    let mut lay2 = Targets::new();
    for e in g.edges.iter().filter(|e| e.tunnel) {
        let Some(secs) = stored.edges.get(&e.id.0) else {
            continue;
        };
        let ground = |x: f64, z: f64| pkg.surface_height(x, z);
        let samples = tn::probe(e, &ground);
        let runs = tn::exposed_runs(&samples, secs);
        if runs.is_empty() {
            continue;
        }
        let mut secs = secs.clone();
        for &(a, b, _) in &runs {
            secs = carve_open(&secs, a - tn::STEP_M, b + tn::STEP_M);
            st.cut_open_m += b - a + tn::STEP_M;
        }
        for x in &samples {
            if section_kind_at(&secs, x.s) == SectionKind::Open {
                lay_band(
                    x,
                    band_of(SectionKind::Open, x, e.width_m, sp),
                    sp,
                    &mut lay2,
                );
            }
        }
        stored.edges.insert(e.id.0, secs);
    }
    apply(pkg, g, &roads, &Targets::new(), &lay2, &mut st);
    pkg.look.tunnels = Some(stored);
    st
}

/// The kind of `secs` at `s` (bored outside them).
fn section_kind_at(secs: &[Section], s: f64) -> SectionKind {
    secs.iter()
        .find(|k| s >= k.from && s <= k.to)
        .map_or(SectionKind::Bored, |k| k.kind)
}

/// Global grid vertices within `r` of `p` (a square).
fn vertices_near(p: [f64; 2], r: f64, sp: f64) -> impl Iterator<Item = (i64, i64)> {
    let (x0, x1) = (
        ((p[0] - r) / sp).floor() as i64,
        ((p[0] + r) / sp).ceil() as i64,
    );
    let (z0, z1) = (
        ((p[1] - r) / sp).floor() as i64,
        ((p[1] + r) / sp).ceil() as i64,
    );
    (z0..=z1).flat_map(move |z| (x0..=x1).map(move |x| (x, z)))
}

/// The build check: every tunnel's bored stretches still exposed after
/// shaping (osm way, from, to, worst m, where).
pub fn check(pkg: &WorldPackage, tags: &TunnelTags) -> Vec<(i64, f64, f64, f64, [f64; 2])> {
    let Some(g) = &pkg.graph else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in g.edges.iter().filter(|e| e.tunnel) {
        let tag = tags.get(&e.osm_way).map(String::as_str);
        let (samples, secs) = edge_sections(pkg, e, tag);
        for (a, b, w) in tn::exposed_runs(&samples, &secs) {
            let at = samples
                .iter()
                .find(|x| x.s >= a)
                .map_or(e.points[0], |x| x.p);
            out.push((e.osm_way, a, b, w, at));
        }
    }
    out
}

/// The report of [`measure`] and [`buried_roads`] (worst first).
pub fn format_report(
    pkg: &WorldPackage,
    m: &[TunnelMeasure],
    buried: &[(usize, f64, f64, f64)],
) -> String {
    use std::fmt::Write;
    let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let ll = |p: [f64; 2]| {
        let (la, lo) = proj.unproject(p);
        format!("{la:.5},{lo:.5} (x {:.0} z {:.0})", p[0], p[1])
    };
    let mut s = String::new();
    let sum = |f: &dyn Fn(&TunnelMeasure) -> f64| m.iter().map(f).sum::<f64>();
    let _ = writeln!(
        s,
        "tunnels: {} edges, {:.0} m; old tube {:.0} m, of it exposed {:.0} m ({} tunnels); floating {:.0} m; sections: bored {:.0} m, gallery {:.0} m, open {:.0} m; still exposed {:.0} m ({} tunnels, worst {:.1} m)",
        m.len(),
        sum(&|t| t.length_m),
        sum(&|t| t.covered_m),
        sum(&|t| t.old_exposed_m),
        m.iter().filter(|t| t.old_exposed_m > 0.0).count(),
        sum(&|t| t.floating_m),
        sum(&|t| t.bored_m),
        sum(&|t| t.gallery_m),
        sum(&|t| t.open_m),
        sum(&|t| t.exposed_m),
        m.iter().filter(|t| t.exposed_m > 0.0).count(),
        m.iter().map(|t| t.exposed_worst_m).fold(0.0, f64::max),
    );
    let mut tags: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
    for t in m {
        let e = tags.entry(&t.tag).or_default();
        e.0 += 1;
        e.1 += t.length_m;
    }
    let _ = writeln!(
        s,
        "tunnel tags: {}",
        tags.iter()
            .map(|(k, (n, l))| format!("{k} {n} ({l:.0} m)"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut order: Vec<&TunnelMeasure> = m.iter().collect();
    order.sort_by(|a, b| {
        (b.old_exposed_m * b.worst_m.max(0.1))
            .total_cmp(&(a.old_exposed_m * a.worst_m.max(0.1)))
            .then(a.osm_way.cmp(&b.osm_way))
    });
    for t in order {
        let secs: Vec<String> = t
            .sections
            .iter()
            .map(|x| {
                let k = match x.kind {
                    SectionKind::Bored => "B",
                    SectionKind::Gallery { hill_right: true } => "Gr",
                    SectionKind::Gallery { hill_right: false } => "Gl",
                    SectionKind::Open => "O",
                };
                format!("{k}{:.0}", x.to - x.from)
            })
            .collect();
        let _ = writeln!(
            s,
            "tunnel way {} {:?} {}: {:.0} m, old tube {:.0} m exposed {:.0} m worst {:.1} m at {} dir {:.2},{:.2}; cover min {:.1} median {:.1} m; floating {:.0} m; [{}]; still exposed {:.0} m ({:.1} m)",
            t.osm_way,
            t.name.as_deref().unwrap_or(""),
            t.tag,
            t.length_m,
            t.covered_m,
            t.old_exposed_m,
            t.worst_m,
            ll(t.worst_at),
            t.worst_dir[0],
            t.worst_dir[1],
            t.min_cover_m,
            t.median_cover_m,
            t.floating_m,
            secs.join(" "),
            t.exposed_m,
            t.exposed_worst_m,
        );
    }
    if let Some(g) = &pkg.graph {
        let total: f64 = buried.iter().map(|b| b.2 - b.1).sum();
        let _ = writeln!(
            s,
            "buried roads (terrain over a non-tunnel road centre by 0.5 m over 10 m): {} stretches, {total:.0} m",
            buried.len()
        );
        let mut b: Vec<_> = buried.to_vec();
        b.sort_by(|x, y| ((y.2 - y.1) * y.3).total_cmp(&((x.2 - x.1) * x.3)));
        for (ei, a, bb, w) in b.iter().take(30) {
            let e = &g.edges[*ei];
            let (p, _) = tn::point_dir(&e.points, (a + bb) / 2.0);
            let _ = writeln!(
                s,
                "buried road way {} {:?} {:?}: {:.0} m, up to {:.1} m under the terrain, at {}",
                e.osm_way,
                e.name.as_deref().unwrap_or(""),
                e.class,
                bb - a,
                w,
                ll(p)
            );
        }
    }
    s
}

#[cfg(test)]
mod tests;
