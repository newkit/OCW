//! Tunnels of the road graph against the terrain (spec 3.2 *Tunnels*,
//! 4.3 *Tunnels and bridges in the terrain*): pure, wasm-safe, shared by
//! the world builder (shaping the terrain, the `tunnel.*` check) and the
//! renderer (what it draws for each tunnel).
//!
//! A road tagged `tunnel=*` in OSM keeps the natural terrain above it, and
//! the renderer draws a tube there (floor, walls, arch). Where the terrain
//! does not cover the tube – the DEM over a shallow tunnel lower than the
//! roof, a short covered section beside the slope, an avalanche gallery on
//! a valley side, the approach to a portal mapped as part of the tunnel –
//! the tube stood in the open (Andorra, 2026-11-28). This module measures
//! every tunnel along its axis ([`probe`]) and splits it into
//! [`Section`]s of a [`SectionKind`]:
//!
//! - **Bored:** the terrain covers the roof (the tube with its portals),
//! - **Gallery:** the hill on one side reaches over the wall, the other
//!   side is open: a concrete roof on pillars on the valley side,
//! - **Open:** neither side is covered: the road in the open or in a
//!   cutting (no roof).
//!
//! The world builder raises the terrain over shallow stretches (a mound
//! into the slope, [`mound_target`]) so they become bored, fills it under
//! the open side of galleries and open stretches, and fails the build on
//! a bored stretch whose roof is still exposed ([`exposed_runs`]).

use std::collections::BTreeMap;

use crate::graph::Edge;
use crate::package::Bytes;

/// Tube cross-section (shared with the renderer's tube): walls this high
/// above the road, the arch's crown at [`CROWN_M`], the walls [`SIDE_M`]
/// beside the road edge.
pub const WALL_M: f64 = 4.0;
pub const CROWN_M: f64 = 6.0;
pub const SIDE_M: f64 = 1.0;
/// Thickness of the lining over the roof (m): the shell must be covered.
pub const LINING_M: f64 = 0.4;
/// Rock the builder keeps over a bored roof (m): a mound raises the
/// terrain to at least this over the lining.
pub const COVER_M: f64 = 2.0;
/// Sampling step along a tunnel (m).
pub const STEP_M: f64 = 2.0;
/// A bored stretch whose shell sticks out by more than this (m) is
/// exposed (the build check).
pub const EXPOSED_TOL_M: f64 = 0.5;
/// Exposed runs shorter than this (m) are not reported (a 5 m DEM cell
/// at a ridge, the portal's slope).
pub const EXPOSED_MIN_RUN_M: f64 = 6.0;
/// A shallow stretch is raised to a mound when at most this much terrain
/// is missing over the shell at the centre (m); with more missing the
/// stretch is no tunnel in the terrain (a gallery or the open road).
pub const MOUND_MAX_M: f64 = 6.0;
/// Gallery: the hill side's terrain at least this high above the road
/// beside the tube (m) …
pub const GALLERY_HILL_M: f64 = WALL_M;
/// … while the other side is at most this high above the road (m).
pub const GALLERY_OPEN_M: f64 = 1.5;
/// Gallery: roof slab underside and thickness above the road (m), pillar
/// spacing along the road (m).
pub const GALLERY_ROOF_M: f64 = 5.2;
pub const GALLERY_SLAB_M: f64 = 0.6;
pub const GALLERY_PILLAR_STEP_M: f64 = 6.0;
/// A box tunnel: the clear height under its flat roof and the roof
/// slab's thickness (m).
pub const BOX_ROOF_M: f64 = 4.7;
pub const BOX_SLAB_M: f64 = 0.5;
/// Sections shorter than this (m) join their neighbours.
pub const MIN_SECTION_M: f64 = 12.0;
/// Lateral distance beyond the tube's side where the hill side and the
/// open side are judged (m).
const SIDE_PROBE_M: f64 = 2.0;

/// Half the tube's width for a road `width` wide (m).
pub fn half_width(width: f64) -> f64 {
    width / 2.0 + SIDE_M
}

/// Height of the tube's inner roof above the road at lateral offset `l`
/// (m): the walls up to [`WALL_M`], a half ellipse up to [`CROWN_M`].
pub fn roof_height(l: f64, half: f64) -> f64 {
    let t = (l / half).clamp(-1.0, 1.0);
    WALL_M + (CROWN_M - WALL_M) * (1.0 - t * t).max(0.0).sqrt()
}

/// Lateral offsets (fractions of the half width) where the roof's cover
/// is measured.
const ROOF_PROBES: [f64; 5] = [-1.0, -0.5, 0.0, 0.5, 1.0];

/// One sample along a tunnel's axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisSample {
    /// Distance from the edge's start (m).
    pub s: f64,
    /// Road centre (x, z) and its height.
    pub p: [f64; 2],
    pub road_y: f64,
    /// Unit direction (x, z); right = (−dz, dx) as `RoutePoint::right`.
    pub dir: [f64; 2],
    /// Half the tube's width (m).
    pub half: f64,
    /// Terrain minus the top of the lining at the [`ROOF_PROBES`] (m;
    /// negative: the shell sticks out of the terrain by that much).
    pub cover: [f64; 5],
    /// Terrain above the road at the centre (m).
    pub centre: f64,
    /// Terrain above the road [`SIDE_PROBE_M`] beside the tube, left and
    /// right (m).
    pub side: [f64; 2],
}

impl AxisSample {
    /// The least cover over the shell (m).
    pub fn roof_cover(&self) -> f64 {
        self.cover.iter().copied().fold(f64::INFINITY, f64::min)
    }

    /// The least cover over a box's roof slab (m; [`BOX_ROOF_M`] +
    /// [`BOX_SLAB_M`] above the road).
    pub fn box_cover(&self) -> f64 {
        ROOF_PROBES
            .iter()
            .zip(self.cover)
            .map(|(f, c)| {
                c + roof_height(f * self.half, self.half) + LINING_M - BOX_ROOF_M - BOX_SLAB_M
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// The least cover over the roof of a section of `kind` (m).
    pub fn cover_for(&self, kind: SectionKind) -> f64 {
        match kind {
            SectionKind::Box => self.box_cover(),
            _ => self.roof_cover(),
        }
    }

    /// Point at lateral offset `l` (m, positive right).
    pub fn at(&self, l: f64) -> [f64; 2] {
        [self.p[0] - self.dir[1] * l, self.p[1] + self.dir[0] * l]
    }
}

/// Polyline helpers: point and unit direction at arc length `d`.
pub fn point_dir(points: &[[f64; 2]], d: f64) -> ([f64; 2], [f64; 2]) {
    let mut acc = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len <= 1e-9 {
            continue;
        }
        let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        if acc + len >= d {
            let t = ((d - acc) / len).clamp(0.0, 1.0);
            return ([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], dir);
        }
        acc += len;
    }
    let n = points.len();
    let (a, b) = (points[n.saturating_sub(2)], points[n - 1]);
    let len = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-9);
    (b, [(b[0] - a[0]) / len, (b[1] - a[1]) / len])
}

/// Samples `edge` every [`STEP_M`] (both ends included) against `ground`
/// (terrain height at x, z). Empty without a profile or ground.
pub fn probe(edge: &Edge, ground: &dyn Fn(f64, f64) -> Option<f64>) -> Vec<AxisSample> {
    if edge.profile.is_empty() || edge.points.len() < 2 {
        return Vec::new();
    }
    let len = edge.length_m();
    let n = (len / STEP_M).ceil().max(1.0) as usize;
    let half = half_width(edge.width_m);
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let s = (i as f64 * STEP_M).min(len);
        let (p, dir) = point_dir(&edge.points, s);
        let Some(road_y) = edge.height_at(s) else {
            continue;
        };
        let mut x = AxisSample {
            s,
            p,
            road_y,
            dir,
            half,
            cover: [0.0; 5],
            centre: 0.0,
            side: [0.0; 2],
        };
        let h = |l: f64| ground(p[0] - dir[1] * l, p[1] + dir[0] * l);
        let Some(c) = h(0.0) else {
            continue;
        };
        let mut ok = true;
        for (k, f) in ROOF_PROBES.iter().enumerate() {
            let l = f * half;
            match h(l) {
                Some(t) => x.cover[k] = t - (road_y + roof_height(l, half) + LINING_M),
                None => ok = false,
            }
        }
        for (k, sgn) in [-1.0, 1.0].into_iter().enumerate() {
            match h(sgn * (half + SIDE_PROBE_M)) {
                Some(t) => x.side[k] = t - road_y,
                None => ok = false,
            }
        }
        if !ok {
            continue;
        }
        x.centre = c - road_y;
        out.push(x);
    }
    out
}

/// What a stretch of a tunnel is in the terrain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SectionKind {
    /// Under the terrain: the tube.
    Bored,
    /// A gallery: the hill on the given side (−1 left, +1 right), open on
    /// the other.
    Gallery { hill_right: bool },
    /// In the open or in a cutting: no roof.
    Open,
    /// A cut-and-cover box: flat roof at [`BOX_ROOF_M`] (a city tunnel
    /// under streets, an underpass under a road embankment), where the
    /// terrain covers a box but not the tube.
    Box,
}

/// A stretch of a tunnel edge (from, to: distance from the edge's start).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Section {
    pub from: f64,
    pub to: f64,
    pub kind: SectionKind,
}

/// The kind of one sample: a gallery where the hill on one side reaches
/// over the wall beside the tube and the other side is open (the tube's
/// valley wall would stand in the air); bored where the centre is under
/// the terrain and at most [`MOUND_MAX_M`] of cover is missing over the
/// crown (a mound closes the rest); open otherwise.
pub fn sample_kind(x: &AxisSample) -> SectionKind {
    let [left, right] = x.side;
    if right >= GALLERY_HILL_M && left <= GALLERY_OPEN_M {
        return SectionKind::Gallery { hill_right: true };
    }
    if left >= GALLERY_HILL_M && right <= GALLERY_OPEN_M {
        return SectionKind::Gallery { hill_right: false };
    }
    let crown_gap = -x.cover[2]; // missing over the shell at the centre
    if x.centre > 0.5 && crown_gap <= MOUND_MAX_M {
        return SectionKind::Bored;
    }
    SectionKind::Open
}

/// The sections of a tunnel from its samples: the kind of each sample,
/// runs shorter than [`MIN_SECTION_M`] joined to the longer neighbour.
pub fn sections(samples: &[AxisSample]) -> Vec<Section> {
    let mut runs: Vec<Section> = Vec::new();
    for x in samples {
        let k = sample_kind(x);
        match runs.last_mut() {
            Some(r) if r.kind == k => r.to = x.s,
            Some(r) => {
                let mid = (r.to + x.s) / 2.0;
                r.to = mid;
                runs.push(Section {
                    from: mid,
                    to: x.s,
                    kind: k,
                });
            }
            None => runs.push(Section {
                from: x.s,
                to: x.s,
                kind: k,
            }),
        }
    }
    // Join short runs (shortest first) into a neighbour, so a 5 m DEM cell
    // does not split a gallery or a bored tunnel.
    loop {
        if runs.len() < 2 {
            break;
        }
        let Some((i, _)) = runs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.to - r.from < MIN_SECTION_M)
            .min_by(|a, b| (a.1.to - a.1.from).total_cmp(&(b.1.to - b.1.from)))
        else {
            break;
        };
        let into = match (i.checked_sub(1), (i + 1 < runs.len()).then_some(i + 1)) {
            (Some(a), Some(b)) => {
                if runs[a].to - runs[a].from >= runs[b].to - runs[b].from {
                    a
                } else {
                    b
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        let r = runs.remove(i);
        let j = if into > i { into - 1 } else { into };
        runs[j].from = runs[j].from.min(r.from);
        runs[j].to = runs[j].to.max(r.to);
        // Neighbours of the same kind merge.
        let mut k = 0;
        while k + 1 < runs.len() {
            if runs[k].kind == runs[k + 1].kind {
                runs[k].to = runs[k + 1].to;
                runs.remove(k + 1);
            } else {
                k += 1;
            }
        }
    }
    runs
}

/// The sections of `edge` against `ground` ([`probe`] then [`sections`]).
pub fn edge_sections(edge: &Edge, ground: &dyn Fn(f64, f64) -> Option<f64>) -> Vec<Section> {
    sections(&probe(edge, ground))
}

/// Runs (from, to, worst exposure m) of bored stretches whose shell sticks
/// out of the terrain by more than [`EXPOSED_TOL_M`] over at least
/// [`EXPOSED_MIN_RUN_M`], leaving out [`crate::tunnels::PORTAL_SLOPE_M`]
/// inside each end of a bored section (the portal face is cut into the
/// slope there).
pub fn exposed_runs(samples: &[AxisSample], secs: &[Section]) -> Vec<(f64, f64, f64)> {
    let mut out: Vec<(f64, f64, f64)> = Vec::new();
    let mut cur: Option<(f64, f64, f64)> = None;
    for x in samples {
        let roofed = secs.iter().find(|s| {
            matches!(s.kind, SectionKind::Bored | SectionKind::Box)
                && x.s >= s.from + PORTAL_SLOPE_M
                && x.s <= s.to - PORTAL_SLOPE_M
        });
        let e = roofed.map_or(0.0, |s| -x.cover_for(s.kind));
        if roofed.is_some() && e > EXPOSED_TOL_M {
            cur = Some(match cur {
                Some((a, _, w)) => (a, x.s, w.max(e)),
                None => (x.s, x.s, e),
            });
        } else if let Some(c) = cur.take() {
            out.push(c);
        }
    }
    out.extend(cur);
    out.retain(|(a, b, _)| b - a + STEP_M >= EXPOSED_MIN_RUN_M);
    out
}

/// Inside each end of a bored section the portal's slope may lie under
/// the roof (m): the renderer cuts the portal face into the slope there.
pub const PORTAL_SLOPE_M: f64 = 10.0;

/// The terrain height a mound needs at lateral offset `l` from a bored
/// sample (m above sea level), or `None` beyond the mound: the shell
/// covered by [`COVER_M`] over the tube, falling off to the sides at 1:1.5
/// (a natural slope) until it meets the terrain.
pub fn mound_target(x: &AxisSample, l: f64) -> Option<f64> {
    let top = x.road_y + CROWN_M + LINING_M + COVER_M;
    let beyond = (l.abs() - x.half).max(0.0);
    let h = top - beyond / 1.5;
    (h > x.road_y).then_some(h)
}

/// Section tag of the tunnels' sections (written by `world-builder
/// tunnels`; optional, package-wide, small).
pub const TUNNELS_TAG: &[u8; 4] = b"TUNL";
const TUNNELS_VERSION: u8 = 1;

/// The sections of every tunnel edge of a package as the world builder
/// decided them (with the OSM tags the client has not got: galleries
/// tagged `avalanche_protector`, building passages), by stable edge id.
/// The renderer draws these; without the section it classifies from the
/// terrain ([`sections`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TunnelSections {
    pub edges: BTreeMap<u64, Vec<Section>>,
}

impl TunnelSections {
    /// The stored sections of `edge`, else those classified from its
    /// `samples`.
    pub fn sections_of(this: Option<&Self>, edge: &Edge, samples: &[AxisSample]) -> Vec<Section> {
        match this.and_then(|t| t.edges.get(&edge.id.0)) {
            Some(s) => s.clone(),
            None => sections(samples),
        }
    }

    /// The stretches of the tunnel edge `id` whose road lies in the open or
    /// under a gallery's roof (from, to: distance from the edge's start):
    /// there the road is a drawn surface on the shaped terrain like any
    /// other road, so nothing may grow on it (the world-building rule of
    /// 2026-11-28, no plant on a road surface) – unlike a bored or box
    /// stretch, where the terrain above is free. Empty for an edge the
    /// section does not list.
    pub fn surface_spans(&self, id: u64) -> Vec<(f64, f64)> {
        let Some(secs) = self.edges.get(&id) else {
            return Vec::new();
        };
        let mut out: Vec<(f64, f64)> = Vec::new();
        for s in secs {
            if !matches!(s.kind, SectionKind::Open | SectionKind::Gallery { .. }) {
                continue;
            }
            match out.last_mut() {
                Some(last) if (s.from - last.1).abs() < 1e-3 => last.1 = s.to,
                _ => out.push((s.from, s.to)),
            }
        }
        out
    }
}

fn kind_code(k: SectionKind) -> u8 {
    match k {
        SectionKind::Bored => 0,
        SectionKind::Gallery { hill_right: false } => 1,
        SectionKind::Gallery { hill_right: true } => 2,
        SectionKind::Open => 3,
        SectionKind::Box => 4,
    }
}

pub fn write_tunnels(t: &TunnelSections, b: &mut Vec<u8>) {
    b.push(TUNNELS_VERSION);
    b.extend((t.edges.len() as u32).to_le_bytes());
    for (id, secs) in &t.edges {
        b.extend(id.to_le_bytes());
        b.extend((secs.len() as u32).to_le_bytes());
        for s in secs {
            b.extend((s.from as f32).to_le_bytes());
            b.extend((s.to as f32).to_le_bytes());
            b.push(kind_code(s.kind));
        }
    }
}

pub fn read_tunnels(payload: &[u8]) -> Result<TunnelSections, String> {
    let mut r = Bytes::new(payload);
    let version = r.u8()?;
    if version != TUNNELS_VERSION {
        return Err(format!("tunnels layout {version} not supported"));
    }
    let n = r.u32()? as usize;
    let mut edges = BTreeMap::new();
    for _ in 0..n {
        let id = r.u64()?;
        let count = r.u32()? as usize;
        let mut secs = Vec::with_capacity(count.min(1 << 12));
        for _ in 0..count {
            let from = f64::from(r.f32()?);
            let to = f64::from(r.f32()?);
            let kind = match r.u8()? {
                0 => SectionKind::Bored,
                1 => SectionKind::Gallery { hill_right: false },
                2 => SectionKind::Gallery { hill_right: true },
                3 => SectionKind::Open,
                4 => SectionKind::Box,
                k => return Err(format!("tunnels: unknown section kind {k}")),
            };
            secs.push(Section { from, to, kind });
        }
        edges.insert(id, secs);
    }
    r.finish("tunnels")?;
    Ok(TunnelSections { edges })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeId, Oneway, RoadClass, VertexId};

    /// A straight tunnel edge along +x from 0 to `len` at height 100.
    pub(crate) fn edge(len: f64) -> Edge {
        let n = (len / 5.0) as usize;
        Edge {
            id: EdgeId(1),
            from: VertexId(0),
            to: VertexId(1),
            points: vec![[0.0, 0.0], [len, 0.0]],
            class: RoadClass::Secondary,
            name: None,
            width_m: 6.0,
            bridge: false,
            tunnel: true,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 7,
            profile: vec![100.0; n + 1],
        }
    }

    #[test]
    fn a_tunnel_through_a_hill_is_bored_and_covered() {
        let e = edge(400.0);
        // A hill 40 m over the road, its slopes 1:1 at the portals.
        let hill = |x: f64, _z: f64| Some(100.0 + (40.0 - (x - 200.0).abs()).max(-1.0));
        let s = probe(&e, &hill);
        let secs = sections(&s);
        assert!(
            secs.iter().any(|s| s.kind == SectionKind::Bored),
            "{secs:?}"
        );
        assert!(exposed_runs(&s, &secs).is_empty());
    }

    #[test]
    fn a_shallow_tunnel_under_a_low_valley_floor_is_exposed_until_mounded() {
        let e = edge(200.0);
        // The DEM only 3 m over the road: the 6 m tube sticks out.
        let low = |_x: f64, _z: f64| Some(103.0);
        let s = probe(&e, &low);
        let secs = sections(&s);
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].kind, SectionKind::Bored);
        let ex = exposed_runs(&s, &secs);
        assert_eq!(ex.len(), 1, "{ex:?}");
        assert!(ex[0].2 > 3.0, "{ex:?}");
        // The mound covers it.
        let mound = |x: f64, z: f64| {
            let q = AxisSample {
                p: [x, 0.0],
                ..s[0]
            };
            let t = mound_target(&q, z).unwrap_or(f64::NEG_INFINITY);
            Some(t.max(103.0))
        };
        let s2 = probe(&e, &mound);
        assert!(exposed_runs(&s2, &sections(&s2)).is_empty());
    }

    #[test]
    fn a_gallery_on_a_slope_has_its_hill_side() {
        let e = edge(150.0);
        // Rising 1:1 to the right (+z), falling to the left.
        let slope = |_x: f64, z: f64| Some(100.0 + z);
        let secs = edge_sections(&e, &slope);
        assert_eq!(secs.len(), 1, "{secs:?}");
        assert_eq!(secs[0].kind, SectionKind::Gallery { hill_right: true });
        let slope_l = |_x: f64, z: f64| Some(100.0 - z);
        let secs = edge_sections(&e, &slope_l);
        assert_eq!(secs[0].kind, SectionKind::Gallery { hill_right: false });
    }

    #[test]
    fn a_short_underpass_in_the_open_has_no_roof() {
        let e = edge(30.0);
        let flat = |_x: f64, _z: f64| Some(99.7);
        let secs = edge_sections(&e, &flat);
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].kind, SectionKind::Open);
        assert!(exposed_runs(&probe(&e, &flat), &secs).is_empty());
    }

    #[test]
    fn portals_are_where_the_road_enters_the_hill() {
        let e = edge(400.0);
        // Open approach 0..100 (flat below the road), then a hill.
        let g = |x: f64, _z: f64| {
            Some(if x < 100.0 {
                99.7
            } else {
                100.0 + ((x - 100.0) * 0.6).min(30.0)
            })
        };
        let secs = edge_sections(&e, &g);
        assert_eq!(
            secs.first().map(|s| s.kind),
            Some(SectionKind::Open),
            "{secs:?}"
        );
        let bored = secs.iter().find(|s| s.kind == SectionKind::Bored).unwrap();
        // The portal within a few metres of where the terrain reaches the
        // road (x ≈ 101), not at the OSM end.
        assert!((bored.from - 100.0).abs() < 8.0, "{secs:?}");
    }

    #[test]
    fn the_section_round_trips() {
        let mut t = TunnelSections::default();
        t.edges.insert(
            42,
            vec![
                Section {
                    from: 0.0,
                    to: 10.0,
                    kind: SectionKind::Open,
                },
                Section {
                    from: 10.0,
                    to: 90.0,
                    kind: SectionKind::Bored,
                },
                Section {
                    from: 90.0,
                    to: 120.5,
                    kind: SectionKind::Gallery { hill_right: true },
                },
            ],
        );
        t.edges.insert(
            7,
            vec![Section {
                from: 0.0,
                to: 5.0,
                kind: SectionKind::Gallery { hill_right: false },
            }],
        );
        let mut b = Vec::new();
        write_tunnels(&t, &mut b);
        assert_eq!(read_tunnels(&b).unwrap(), t);
        b[0] = 9;
        assert!(read_tunnels(&b).unwrap_err().contains("not supported"));
    }

    #[test]
    fn roof_profile() {
        assert_eq!(roof_height(0.0, 4.0), CROWN_M);
        assert_eq!(roof_height(4.0, 4.0), WALL_M);
        assert_eq!(roof_height(-9.0, 4.0), WALL_M);
    }

    #[test]
    fn deterministic() {
        let e = edge(300.0);
        let g = |x: f64, z: f64| Some(100.0 + (x * 0.05).sin() * 8.0 + z * 0.3);
        assert_eq!(edge_sections(&e, &g), edge_sections(&e, &g));
    }

    #[test]
    fn surface_spans_are_the_open_and_gallery_stretches_joined() {
        let mut t = TunnelSections::default();
        let sec = |from, to, kind| Section { from, to, kind };
        t.edges.insert(
            7,
            vec![
                sec(0.0, 20.0, SectionKind::Open),
                sec(20.0, 60.0, SectionKind::Gallery { hill_right: true }),
                sec(60.0, 200.0, SectionKind::Bored),
                sec(200.0, 230.0, SectionKind::Box),
                sec(230.0, 250.0, SectionKind::Open),
            ],
        );
        assert_eq!(t.surface_spans(7), vec![(0.0, 60.0), (230.0, 250.0)]);
        assert!(t.surface_spans(8).is_empty());
    }
}
