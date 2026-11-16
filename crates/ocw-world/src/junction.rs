//! Smooth junctions (spec §4.8.8, stage J1): one surface where roads meet,
//! computed from the road graph alone, so the renderer (the drawn
//! surface), the server's physics and the client's prediction (the ridden
//! heights) all get the same result on the current packages.
//!
//! - **Patches** at every vertex of degree ≥ 3 on the ground, and at a way
//!   split (degree 2) that bends more than [`MITRE_MAX_BEND_DEG`] or
//!   changes the width: the incident edges (arms) end at a **mouth line**
//!   a few metres out; between the mouths the patch's outline runs along
//!   the edge lines and round **corner arcs** (radius by the smaller road's
//!   class, the §4.8.8 Q4 fallback). Its **height field** meets every arm
//!   at its mouth with the arm's height and slope (C1): an inverse-square
//!   (Shepard) blend of the arms' linear extensions. Where there is a clear
//!   **through road** (§4.8.8 Q1) only the two main arms take part: the
//!   main road keeps its own surface, and the side arms adapt to it over
//!   5–30 m beyond their mouths (a cubic Hermite blend, C1 at both ends).
//! - **Mitres** at the other way splits: the last row of both strips lies
//!   on the bisector (no wedge gap, no overlap).
//! - The patch is meshed as a star from the vertex (homothetic rings of
//!   the outline); [`Patch::height_at`] interpolates on exactly those
//!   triangles, and [`JunctionSet::drawn_height`] on the blended strip rows,
//!   so the ridden height is the drawn height.
//!
//! Patches are computed lazily per vertex and kept ([`JunctionSet`]): a
//! pure function of the vertex's edges, the same in every process.
//! Vertices that cannot be patched (forks below [`MIN_CORNER_DEG`],
//! outlines that are not star-shaped, bridges, tunnels, the region border)
//! keep the overlapping strips of before.

use std::sync::{Arc, OnceLock};

use crate::graph::{Edge, PROFILE_STEP_M, RoadClass, RoadGraph, VertexId};

type P2 = [f64; 2];

/// A mouth lies at most this far from the vertex (m) …
pub const MAX_MOUTH_M: f64 = 25.0;
/// … and at most this share of the edge's length.
pub const MOUTH_SHARE: f64 = 0.45;
/// Arms closer than this (degrees) are a fork the patch cannot hold.
pub const MIN_CORNER_DEG: f64 = 15.0;
/// Two arms of the highest class less than this far from straight
/// (degrees) are a through road (§4.8.8 Q1).
pub const THROUGH_DEV_DEG: f64 = 30.0;
/// Way splits bending at most this much (degrees) get mitred strip ends;
/// more, or a width change, a patch.
pub const MITRE_MAX_BEND_DEG: f64 = 30.0;
/// Way splits bending less than this (degrees) are left as they are.
pub const MITRE_MIN_BEND_DEG: f64 = 0.5;
/// Width changes above this (m) at a way split get a patch.
pub const WIDTH_CHANGE_M: f64 = 0.05;
/// Corner arcs: at most this angle per segment (degrees).
pub const ARC_STEP_DEG: f64 = 10.0;
/// Straight outline pieces and mouth lines are cut into pieces this long.
pub const OUTLINE_STEP_M: f64 = 1.5;
pub const MOUTH_STEP_M: f64 = 1.0;
/// The interior rings of the patch mesh lie about this far apart (m).
pub const RING_M: f64 = 2.5;
/// Side-arm blend: the grade-change rate that sets its length, and its
/// limits (m) (§4.8.8: `L = clamp(|Δh| / 0.01 per m, 5 m, 30 m)`).
pub const BLEND_RATE: f64 = 0.01;
pub const BLEND_MIN_M: f64 = 5.0;
pub const BLEND_MAX_M: f64 = 30.0;
/// Rows of a blended side arm lie at most this far apart (m).
pub const BLEND_ROW_M: f64 = 1.0;
/// Patches with more arms than this are left out (J3: complex junctions).
pub const MAX_ARMS: usize = 8;

/// Corner radius by the smaller road's class (m): the §4.8.8 Q4 fallback
/// for worlds without their §4.8.9 table.
pub fn corner_radius(class: RoadClass) -> f64 {
    match class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => 8.0,
        RoadClass::Tertiary | RoadClass::Unclassified | RoadClass::Residential => 6.0,
        RoadClass::Service | RoadClass::Track => 3.0,
        RoadClass::Cycleway | RoadClass::Path => 1.5,
    }
}

// --- small vector maths -------------------------------------------------------

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: P2, k: f64) -> P2 {
    [a[0] * k, a[1] * k]
}
fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn norm(a: P2) -> f64 {
    a[0].hypot(a[1])
}
fn unit(a: P2) -> P2 {
    let l = norm(a);
    if l > 1e-12 {
        mul(a, 1.0 / l)
    } else {
        [1.0, 0.0]
    }
}
/// The side a strip calls "right" (`ocw-render::road_mesh`: (−z, x)); the
/// polar angle `atan2(z, x)` grows towards it.
pub fn side_pos(d: P2) -> P2 {
    [-d[1], d[0]]
}
fn side_neg(d: P2) -> P2 {
    [d[1], -d[0]]
}
fn xz(p: [f64; 3]) -> P2 {
    [p[0], p[2]]
}
fn at_y(p: P2, y: f64) -> [f64; 3] {
    [p[0], y, p[1]]
}
fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// Distance from `p` to the segment `a`–`b`.
fn seg_dist(p: P2, a: P2, b: P2) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    let t = if l2 > 0.0 {
        (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    norm(sub(add(a, mul(ab, t)), p))
}

/// Cubic Hermite on [0, 1]: values `p0`, `p1`, slopes (per unit `t`) `m0`,
/// `m1`.
fn hermite(t: f64, p0: f64, m0: f64, p1: f64, m1: f64) -> f64 {
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0
        + (t3 - 2.0 * t2 + t) * m0
        + (-2.0 * t3 + 3.0 * t2) * p1
        + (t3 - t2) * m1
}

/// Barycentric height of `p` in the triangle (xz) `t`, if inside (with a
/// small tolerance).
fn tri_height(t: [[f64; 3]; 3], p: P2, tol: f64) -> Option<f64> {
    let (a, b, c) = (xz(t[0]), xz(t[1]), xz(t[2]));
    let d = cross(sub(b, a), sub(c, a));
    if d.abs() < 1e-12 {
        return None;
    }
    let w1 = cross(sub(p, a), sub(c, a)) / d;
    let w2 = cross(sub(b, a), sub(p, a)) / d;
    let w0 = 1.0 - w1 - w2;
    (w0 >= -tol && w1 >= -tol && w2 >= -tol).then(|| w0 * t[0][1] + w1 * t[1][1] + w2 * t[2][1])
}

// --- edge polylines -----------------------------------------------------------

/// Horizontal cumulative lengths of an edge's points.
fn cumulative(points: &[P2]) -> Vec<f64> {
    let mut out = Vec::with_capacity(points.len());
    let mut s = 0.0;
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            s += norm(sub(*p, points[i - 1]));
        }
        out.push(s);
    }
    out
}

/// The point at `s` (m from the edge's `from` vertex, clamped) and the
/// edge's forward direction there (the segment containing `s`; at a point
/// shared by two segments, the later one).
pub fn frame_at(points: &[P2], cum: &[f64], s: f64) -> (P2, P2) {
    let n = points.len();
    if n < 2 {
        return (points[0], [1.0, 0.0]);
    }
    let len = cum[n - 1];
    let s = s.clamp(0.0, len);
    let k = cum.partition_point(|&c| c <= s).clamp(1, n - 1);
    let (a, b) = (points[k - 1], points[k]);
    let seg = cum[k] - cum[k - 1];
    let t = if seg > 1e-12 {
        (s - cum[k - 1]) / seg
    } else {
        0.0
    };
    (add(a, mul(sub(b, a), t)), unit(sub(b, a)))
}

/// Profile height at `s` from the `from` vertex.
fn height(e: &Edge, s: f64) -> f64 {
    e.height_at(s).unwrap_or(0.0)
}

/// Grade of the profile at `s` going towards `+dir` (+1 forward, −1 back):
/// the slope of the profile interval on that side.
fn grade(e: &Edge, s: f64, dir: f64) -> f64 {
    let len = e.length_m();
    let d = 0.05;
    let s2 = (s + dir * d).clamp(0.0, len);
    if (s2 - s).abs() < 1e-9 {
        // At an end: the interval on the other side.
        let s1 = (s - dir * d).clamp(0.0, len);
        return (height(e, s) - height(e, s1)) / d;
    }
    (height(e, s2) - height(e, s)) / (s2 - s).abs()
}

/// The points every [`PROFILE_STEP_M`] along `points`, keeping the
/// vertices, with their distances (as the world builder samples a road
/// strip, `ocw_worldgen::profile::sample_along`).
fn sample_along(points: &[P2]) -> Vec<(P2, f64)> {
    let step = PROFILE_STEP_M;
    let mut out = vec![(points[0], 0.0)];
    let (mut walked, mut next) = (0.0, step);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = norm(sub(b, a));
        while len > 0.0 && next <= walked + len + 1e-9 {
            let t = ((next - walked) / len).min(1.0);
            out.push((add(a, mul(sub(b, a), t)), next));
            next += step;
        }
        walked += len;
        let last = out.last().expect("non-empty").1;
        if len > 0.0 && walked - last > 1e-6 {
            out.push((b, walked));
        }
    }
    let last = *points.last().expect("non-empty");
    let end = out.last().expect("non-empty").0;
    if (end[0] - last[0]).abs() + (end[1] - last[1]).abs() > 1e-6 {
        out.push((last, walked));
    }
    out
}

/// The strip's along coordinate (3D arc length of the drawn points from
/// the edge's start, `RoadDress::start_m`) at `s`.
fn along_at(e: &Edge, s: f64) -> f64 {
    let pts = sample_along(&e.points);
    let mut along = 0.0;
    for w in pts.windows(2) {
        let ((a, sa), (b, sb)) = (w[0], w[1]);
        let (ha, hb) = (height(e, sa), height(e, sb));
        let full = (norm(sub(b, a)).powi(2) + (hb - ha).powi(2)).sqrt();
        if s <= sb {
            let t = if sb > sa { (s - sa) / (sb - sa) } else { 0.0 };
            return along + full * t.clamp(0.0, 1.0);
        }
        along += full;
    }
    along
}

// --- patches ------------------------------------------------------------------

/// One road of a patch.
#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub edge: usize,
    /// The edge leaves the vertex (its `from` end is here).
    pub at_from: bool,
    pub class: RoadClass,
    pub half: f64,
    /// The edge's length (m).
    pub len: f64,
    /// Straight-line direction from the vertex (the first 8 m), outward.
    pub dir: P2,
    /// The mouth: metres from the vertex along the edge, its centre and the
    /// outward direction there.
    pub mouth_m: f64,
    pub centre: P2,
    pub t: P2,
    /// The mouth row in the strip's orientation: [left, right] (x, y, z).
    pub row: [[f64; 3]; 2],
    /// The arm's own height and outward grade at the mouth.
    pub h: f64,
    pub grade: f64,
    /// Side arm of a through road: blended onto the main road's surface over
    /// this many metres beyond the mouth (0: not blended).
    pub blend_m: f64,
    /// The strip's along coordinate at the mouth (dashes, textures).
    pub along: f64,
}

impl Arm {
    /// Metres from the edge's `from` vertex of a point `m` from this vertex.
    pub fn s_edge(&self, m: f64) -> f64 {
        if self.at_from { m } else { self.len - m }
    }

    /// The mouth corner on the side the polar angle shrinks to.
    pub fn p(&self) -> [f64; 3] {
        if self.at_from {
            self.row[0]
        } else {
            self.row[1]
        }
    }

    /// The mouth corner on the side the polar angle grows to.
    pub fn q(&self) -> [f64; 3] {
        if self.at_from {
            self.row[1]
        } else {
            self.row[0]
        }
    }

    /// The arm's surface extended past its mouth: flat across, along its
    /// grade.
    fn extension(&self, p: P2) -> f64 {
        self.h + self.grade * dot(sub(p, self.centre), self.t)
    }

    fn mouth_dist(&self, p: P2) -> f64 {
        seg_dist(p, xz(self.row[0]), xz(self.row[1]))
    }
}

/// A point of a patch outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutlinePoint {
    pub p: [f64; 3],
    /// On the mouth line of this arm (its corners included).
    pub mouth: Option<usize>,
}

/// The outline between two mouths: from arm `from`'s corner `q` to the next
/// arm's corner `p`, along the edge lines and the corner arc.
#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    pub from_arm: usize,
    pub to_arm: usize,
    pub points: Vec<[f64; 3]>,
}

/// The junction surface at one vertex.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub vertex: u32,
    pub centre: [f64; 3],
    /// By polar angle (`atan2(z, x)`, growing).
    pub arms: Vec<Arm>,
    /// The main road's two arms, where there is a clear through road.
    pub through: Option<[usize; 2]>,
    /// Closed, by growing polar angle (star-shaped from the centre).
    pub outline: Vec<OutlinePoint>,
    /// Polar angle of each outline point, unwrapped (growing from the
    /// first).
    angles: Vec<f64>,
    pub chains: Vec<Chain>,
    /// Ring scales (1 = the outline, decreasing; the centre is the last
    /// vertex of the mesh).
    rings: Vec<f64>,
    /// Ring j point k is `vertices[j * outline.len() + k]`.
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub min: P2,
    pub max: P2,
}

impl Patch {
    fn n(&self) -> usize {
        self.outline.len()
    }

    /// The patch's own height field at `p` (exact, not the mesh).
    pub fn field(&self, p: P2) -> f64 {
        match self.through {
            Some([a, b]) => shepard([&self.arms[a], &self.arms[b]].into_iter(), p),
            None => shepard(self.arms.iter(), p),
        }
    }

    /// The main road's surface (the through road's; else the field).
    pub fn main_surface(&self, p: P2) -> f64 {
        self.field(p)
    }

    /// The arm whose surface, colour and frame the patch takes: the first
    /// of the through road, else the widest.
    pub fn main_arm(&self) -> usize {
        match self.through {
            Some([a, _]) => a,
            None => (0..self.arms.len())
                .max_by(|&a, &b| {
                    self.arms[a]
                        .half
                        .total_cmp(&self.arms[b].half)
                        .then(self.arms[b].class.cmp(&self.arms[a].class))
                })
                .unwrap_or(0),
        }
    }

    /// The drawn surface at `p` (on the patch's triangles); `None` outside
    /// the patch.
    pub fn height_at(&self, p: P2) -> Option<f64> {
        let n = self.n();
        let c = xz(self.centre);
        let v = sub(p, c);
        if norm(v) < 1e-9 {
            return Some(self.centre[1]);
        }
        if p[0] < self.min[0] - 0.01
            || p[1] < self.min[1] - 0.01
            || p[0] > self.max[0] + 0.01
            || p[1] > self.max[1] + 0.01
        {
            return None;
        }
        let tau = std::f64::consts::TAU;
        let a0 = self.angles[0];
        let mut th = v[1].atan2(v[0]);
        while th < a0 {
            th += tau;
        }
        while th >= a0 + tau {
            th -= tau;
        }
        let k = self.angles.partition_point(|&a| a <= th).clamp(1, n) - 1;
        let k1 = (k + 1) % n;
        let (ba, bb) = (xz(self.outline[k].p), xz(self.outline[k1].p));
        let (a, b) = (sub(ba, c), sub(bb, c));
        let (ca, cb) = (cross(v, a), cross(v, b));
        let lam = if (ca - cb).abs() > 1e-15 {
            (ca / (ca - cb)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = add(a, mul(sub(b, a), lam));
        let t = norm(v) / norm(q).max(1e-12);
        if t > 1.0 + 1e-7 {
            return None;
        }
        let nr = self.rings.len();
        let vert = |j: usize, i: usize| {
            if j >= nr {
                self.vertices[nr * n]
            } else {
                self.vertices[j * n + i]
            }
        };
        // Ring band j (ring j outside, j + 1 inside; past the last ring the
        // centre).
        let j = self
            .rings
            .partition_point(|&r| r > t)
            .saturating_sub(1)
            .min(nr - 1);
        for jj in [j, j.saturating_sub(1), (j + 1).min(nr - 1)] {
            let (o0, o1, i0, i1) = (vert(jj, k), vert(jj, k1), vert(jj + 1, k), vert(jj + 1, k1));
            for tri in [[o0, o1, i1], [o0, i1, i0]] {
                if let Some(h) = tri_height(tri, p, 1e-7) {
                    return Some(h);
                }
            }
        }
        // On a boundary within rounding: the nearest vertex's band.
        for tri in &self.triangles {
            let t3 = tri.map(|i| self.vertices[i as usize]);
            if let Some(h) = tri_height(t3, p, 1e-5) {
                return Some(h);
            }
        }
        None
    }

    /// Whether `p` lies inside the outline.
    pub fn contains(&self, p: P2) -> bool {
        self.height_at(p).is_some()
    }

    /// Texture coordinates (across, along) at `p` in the main arm's strip
    /// frame, continued across the patch (blended into the second arm's
    /// line for a through road).
    pub fn uv(&self, p: P2) -> [f64; 2] {
        let a = &self.arms[self.main_arm()];
        let rho = if a.at_from { 1.0 } else { -1.0 };
        let strip_dir = mul(a.t, rho);
        let ua = dot(sub(p, a.centre), side_pos(strip_dir));
        let va = a.along + rho * dot(sub(p, a.centre), a.t);
        let Some([ia, ib]) = self.through else {
            return [ua, va];
        };
        let b = &self.arms[ib];
        let c = xz(self.centre);
        let vc = a.along + rho * dot(sub(c, a.centre), a.t);
        let dir_b = mul(b.dir, -rho);
        let ub = dot(sub(p, c), side_pos(dir_b));
        let vb = vc - rho * dot(sub(p, c), b.dir);
        let (da, db) = (
            self.arms[ia].mouth_dist(p).max(1e-9),
            b.mouth_dist(p).max(1e-9),
        );
        let (wa, wb) = (1.0 / (da * da), 1.0 / (db * db));
        [
            (wa * ua + wb * ub) / (wa + wb),
            (wa * va + wb * vb) / (wa + wb),
        ]
    }

    /// The sides of the main arm's strip frame (0 its left, u < 0; 1 its
    /// right) on which other arms leave: there the main road's edge line
    /// breaks (§4.8.8 Q2 fallback). Both for a patch without a through road.
    pub fn open_sides(&self) -> [bool; 2] {
        let Some([ia, ib]) = self.through else {
            return [true, true];
        };
        let mut out = [false, false];
        for (i, arm) in self.arms.iter().enumerate() {
            if i == ia || i == ib {
                continue;
            }
            let u = self.uv(arm.centre)[0];
            out[usize::from(u > 0.0)] = true;
        }
        out
    }
}

/// The inverse-square blend of the arms' extensions (weights by distance
/// to each mouth line): C1 at every mouth.
fn shepard<'a>(arms: impl Iterator<Item = &'a Arm>, p: P2) -> f64 {
    let (mut sw, mut sh) = (0.0, 0.0);
    for a in arms {
        let d = a.mouth_dist(p);
        if d < 1e-9 {
            return a.extension(p);
        }
        let w = 1.0 / (d * d);
        sw += w;
        sh += w * a.extension(p);
    }
    if sw > 0.0 { sh / sw } else { 0.0 }
}

/// Polar angle of `p` around `c`.
fn angle(c: P2, p: P2) -> f64 {
    let v = sub(p, c);
    v[1].atan2(v[0])
}

/// Intersection of the lines `a + s da` and `b + t db`: (point, s, t).
fn intersect(a: P2, da: P2, b: P2, db: P2) -> Option<(P2, f64, f64)> {
    let den = cross(da, db);
    if den.abs() < 1e-9 {
        return None;
    }
    let w = sub(b, a);
    let s = cross(w, db) / den;
    let t = cross(w, da) / den;
    Some((add(a, mul(da, s)), s, t))
}

/// A concave corner between arm `i` (its +side edge line) and arm `j`
/// (its −side edge line), the angle `phi` from `i` to `j`.
struct CornerArc {
    /// Tangent points, as distances from the vertex along each arm's line.
    si: f64,
    sj: f64,
    centre: P2,
    radius: f64,
    ti: P2,
    tj: P2,
}

fn build_patch(g: &RoadGraph, adj: &[usize], v: usize) -> Option<Patch> {
    let c = g.vertices[v];
    // Arms.
    struct Raw {
        edge: usize,
        at_from: bool,
        dir: P2,
        half: f64,
        len: f64,
        class: RoadClass,
        cum: Vec<f64>,
    }
    let mut raws: Vec<Raw> = Vec::new();
    for &ei in adj {
        let e = &g.edges[ei];
        let len = e.length_m();
        if len < 1.0 || e.profile.is_empty() {
            return None;
        }
        let at_from = e.from.0 as usize == v;
        let cum = cumulative(&e.points);
        let d = 8.0f64.min(MOUTH_SHARE * len);
        let s = if at_from { d } else { len - d };
        let (p, _) = frame_at(&e.points, &cum, s);
        raws.push(Raw {
            edge: ei,
            at_from,
            dir: unit(sub(p, c)),
            half: e.width_m / 2.0,
            len,
            class: e.class,
            cum,
        });
    }
    raws.sort_by(|a, b| {
        a.dir[1]
            .atan2(a.dir[0])
            .total_cmp(&b.dir[1].atan2(b.dir[0]))
            .then(a.edge.cmp(&b.edge))
    });
    let n = raws.len();
    let tau = std::f64::consts::TAU;
    // The angle from arm i to the next.
    let gap = |i: usize| {
        let (a, b) = (raws[i].dir, raws[(i + 1) % n].dir);
        let mut d = b[1].atan2(b[0]) - a[1].atan2(a[0]);
        while d <= 0.0 {
            d += tau;
        }
        d
    };
    if (0..n).any(|i| gap(i) < MIN_CORNER_DEG.to_radians()) {
        return None;
    }
    let caps: Vec<f64> = raws
        .iter()
        .map(|r| MAX_MOUTH_M.min(MOUTH_SHARE * r.len))
        .collect();
    // Concave corners.
    let mut corners: Vec<Option<CornerArc>> = Vec::with_capacity(n);
    let mut need = vec![1.0f64; n];
    for (i, r) in raws.iter().enumerate() {
        let other = raws
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != i)
            .map(|(_, o)| o.half)
            .fold(0.0, f64::max);
        need[i] = need[i].max(other).max(r.half.min(1.0));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let phi = gap(i);
        if phi >= std::f64::consts::PI - 0.05 {
            corners.push(None);
            continue;
        }
        let (ri, rj) = (&raws[i], &raws[j]);
        let li = add(c, mul(side_pos(ri.dir), ri.half));
        let lj = add(c, mul(side_neg(rj.dir), rj.half));
        let hit = intersect(li, ri.dir, lj, rj.dir)
            .filter(|&(_, si, sj)| si <= caps[i] - 0.2 && sj <= caps[j] - 0.2);
        let Some((x, sx_i, sx_j)) = hit else {
            // Nearly straight (a width change): the edge lines meet far
            // out, the outline runs straight across; an acute corner that
            // far out is a fork the patch cannot hold.
            if phi > 2.0 {
                corners.push(None);
                continue;
            }
            return None;
        };
        let small = ri.class.max(rj.class);
        let half_phi = phi / 2.0;
        let mut tlen = corner_radius(small) / half_phi.tan();
        tlen = tlen.min(caps[i] - sx_i).min(caps[j] - sx_j).max(0.0);
        let radius = tlen * half_phi.tan();
        let bis = unit(add(ri.dir, rj.dir));
        let centre = add(x, mul(bis, radius / half_phi.sin()));
        let (si, sj) = (sx_i + tlen, sx_j + tlen);
        need[i] = need[i].max(si);
        need[j] = need[j].max(sj);
        corners.push(Some(CornerArc {
            si,
            sj,
            centre,
            radius,
            ti: add(x, mul(ri.dir, tlen)),
            tj: add(x, mul(rj.dir, tlen)),
        }));
    }
    // Mouths.
    let mut arms: Vec<Arm> = Vec::with_capacity(n);
    for (i, r) in raws.iter().enumerate() {
        let m = need[i].min(caps[i]);
        let e = &g.edges[r.edge];
        let s = if r.at_from { m } else { r.len - m };
        let (cp, fwd) = frame_at(&e.points, &r.cum, s);
        let t = if r.at_from { fwd } else { mul(fwd, -1.0) };
        let h = height(e, s);
        let right = side_pos(fwd);
        let row = [
            at_y(sub(cp, mul(right, r.half)), h),
            at_y(add(cp, mul(right, r.half)), h),
        ];
        arms.push(Arm {
            edge: r.edge,
            at_from: r.at_from,
            class: r.class,
            half: r.half,
            len: r.len,
            dir: r.dir,
            mouth_m: m,
            centre: cp,
            t,
            row,
            h,
            grade: grade(e, s, if r.at_from { 1.0 } else { -1.0 }),
            blend_m: 0.0,
            along: along_at(e, s),
        });
    }
    // Through road: two arms of the highest class, nearly straight; only
    // one such pair.
    let best = arms.iter().map(|a| a.class).min()?;
    let mut pairs = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            if arms[i].class != best || arms[j].class != best {
                continue;
            }
            let dev = std::f64::consts::PI - dot(arms[i].dir, arms[j].dir).clamp(-1.0, 1.0).acos();
            if dev < THROUGH_DEV_DEG.to_radians() {
                pairs.push([i, j]);
            }
        }
    }
    let through = (pairs.len() == 1).then(|| pairs[0]);
    let vh = {
        let e = &g.edges[arms[0].edge];
        height(e, if arms[0].at_from { 0.0 } else { arms[0].len })
    };
    let mut patch = Patch {
        vertex: v as u32,
        centre: at_y(c, vh),
        arms,
        through,
        outline: Vec::new(),
        angles: Vec::new(),
        chains: Vec::new(),
        rings: Vec::new(),
        vertices: Vec::new(),
        triangles: Vec::new(),
        min: c,
        max: c,
    };
    // Side arms of a through road: their mouth rows on the main surface,
    // and the blend beyond.
    if let Some([a, b]) = through {
        for i in 0..n {
            if i == a || i == b {
                continue;
            }
            let row = patch.arms[i].row.map(|q| at_y(xz(q), patch.field(xz(q))));
            let mid = lerp3(row[0], row[1], 0.5);
            let dh = (mid[1] - patch.arms[i].h).abs();
            let arm = &patch.arms[i];
            let room = 0.5 * arm.len - arm.mouth_m;
            let mut l = (dh / BLEND_RATE).clamp(BLEND_MIN_M, BLEND_MAX_M).min(room);
            if l < 1.0 {
                l = 0.0;
            }
            let arm = &mut patch.arms[i];
            arm.row = row;
            arm.blend_m = l;
        }
    }
    // The outline: per arm its mouth (P to Q), then the chain to the next.
    let mut outline: Vec<OutlinePoint> = Vec::new();
    let mut chains = Vec::with_capacity(n);
    for i in 0..n {
        let j = (i + 1) % n;
        let arm = &patch.arms[i];
        let (p, q) = (arm.p(), arm.q());
        let pieces = (norm(sub(xz(q), xz(p))) / MOUTH_STEP_M).ceil().max(1.0) as usize;
        for k in 0..=pieces {
            outline.push(OutlinePoint {
                p: lerp3(p, q, k as f64 / pieces as f64),
                mouth: Some(i),
            });
        }
        // The chain from Q_i to P_j.
        let next = &patch.arms[j];
        let mut pts2: Vec<P2> = vec![xz(q)];
        match &corners[i] {
            Some(cn) => {
                if cn.si < arm.mouth_m - 0.05 {
                    pts2.push(cn.ti);
                }
                let a0 = angle(cn.centre, cn.ti);
                let mut a1 = angle(cn.centre, cn.tj);
                // The short way round.
                while a1 - a0 > std::f64::consts::PI {
                    a1 -= tau;
                }
                while a0 - a1 > std::f64::consts::PI {
                    a1 += tau;
                }
                let steps = ((a1 - a0).abs() / ARC_STEP_DEG.to_radians())
                    .ceil()
                    .max(1.0) as usize;
                for k in 1..steps {
                    let a = a0 + (a1 - a0) * k as f64 / steps as f64;
                    pts2.push(add(cn.centre, [cn.radius * a.cos(), cn.radius * a.sin()]));
                }
                if cn.radius > 1e-6 || pts2.len() == 1 {
                    pts2.push(cn.tj);
                }
                if cn.sj >= next.mouth_m - 0.05 {
                    pts2.pop();
                }
            }
            None => {
                // Straight or convex: through the edge lines' crossing where
                // it lies between the mouths.
                let li = add(c, mul(side_pos(arm.dir), arm.half));
                let lj = add(c, mul(side_neg(next.dir), next.half));
                if let Some((x, si, sj)) = intersect(li, arm.dir, lj, next.dir)
                    && si < arm.mouth_m - 0.05
                    && sj < next.mouth_m - 0.05
                    && si > -arm.mouth_m.max(next.half)
                    && sj > -next.mouth_m.max(arm.half)
                {
                    pts2.push(x);
                } else if arm.half > next.half + WIDTH_CHANGE_M {
                    // A width change: the wider road keeps its width up to
                    // the vertex, the taper lies beyond it.
                    pts2.push(li);
                } else if next.half > arm.half + WIDTH_CHANGE_M {
                    pts2.push(lj);
                }
            }
        }
        pts2.push(xz(next.p()));
        // Densify the straight pieces.
        let mut dense: Vec<P2> = vec![pts2[0]];
        for w in pts2.windows(2) {
            let k = (norm(sub(w[1], w[0])) / OUTLINE_STEP_M).ceil().max(1.0) as usize;
            for s in 1..=k {
                dense.push(add(w[0], mul(sub(w[1], w[0]), s as f64 / k as f64)));
            }
        }
        let last = dense.len() - 1;
        let pts3: Vec<[f64; 3]> = dense
            .iter()
            .enumerate()
            .map(|(k, &p2)| {
                if k == 0 {
                    q
                } else if k == last {
                    next.p()
                } else {
                    at_y(p2, patch.field(p2))
                }
            })
            .collect();
        for &p3 in &pts3[1..last] {
            outline.push(OutlinePoint { p: p3, mouth: None });
        }
        chains.push(Chain {
            from_arm: i,
            to_arm: j,
            points: pts3,
        });
    }
    // Star-shaped from the centre: polar angles strictly growing, once round.
    let mut angles = Vec::with_capacity(outline.len());
    let mut prev = angle(c, xz(outline[0].p));
    angles.push(prev);
    for o in &outline[1..] {
        if norm(sub(xz(o.p), c)) < 0.3 {
            return None;
        }
        let mut a = angle(c, xz(o.p));
        while a <= prev - 1e-12 {
            a += tau;
        }
        if a - prev < 1e-6 || a - prev > std::f64::consts::PI {
            return None;
        }
        angles.push(a);
        prev = a;
    }
    if prev - angles[0] >= tau - 1e-6 {
        return None;
    }
    let first = angles[0] + tau;
    if first - prev < 1e-6 || first - prev > std::f64::consts::PI {
        return None;
    }
    patch.outline = outline;
    patch.angles = angles;
    patch.chains = chains;
    mesh(&mut patch);
    Some(patch)
}

/// The star mesh: rings of the outline scaled towards the centre.
fn mesh(patch: &mut Patch) {
    let c = xz(patch.centre);
    let n = patch.outline.len();
    let reach = patch
        .outline
        .iter()
        .map(|o| norm(sub(xz(o.p), c)))
        .fold(0.0, f64::max);
    let nr = ((reach / RING_M).ceil() as usize).clamp(2, 10);
    patch.rings = (0..nr).map(|j| 1.0 - j as f64 / nr as f64).collect();
    let mut verts = Vec::with_capacity(nr * n + 1);
    for (j, &r) in patch.rings.iter().enumerate() {
        for o in &patch.outline {
            if j == 0 {
                verts.push(o.p);
            } else {
                let p2 = add(c, mul(sub(xz(o.p), c), r));
                verts.push(at_y(p2, patch.field(p2)));
            }
        }
    }
    let cy = patch.field(c);
    patch.centre[1] = cy;
    verts.push(patch.centre);
    let (mut lo, mut hi) = (c, c);
    for o in &patch.outline {
        lo = [lo[0].min(o.p[0]), lo[1].min(o.p[2])];
        hi = [hi[0].max(o.p[0]), hi[1].max(o.p[2])];
    }
    patch.min = lo;
    patch.max = hi;
    let idx = |j: usize, k: usize| -> u32 {
        if j >= nr {
            (nr * n) as u32
        } else {
            (j * n + k) as u32
        }
    };
    let mut tris = Vec::with_capacity(2 * n * nr);
    for j in 0..nr {
        for k in 0..n {
            let k1 = (k + 1) % n;
            let (o0, o1, i0, i1) = (idx(j, k), idx(j, k1), idx(j + 1, k), idx(j + 1, k1));
            tris.push([o0, o1, i1]);
            if j + 1 < nr {
                tris.push([o0, i1, i0]);
            }
        }
    }
    // Facing up (as the road strips: cross(b − a, c − a).y > 0).
    for t in &mut tris {
        let [a, b, cc] = t.map(|i| verts[i as usize]);
        let up = (b[2] - a[2]) * (cc[0] - a[0]) - (b[0] - a[0]) * (cc[2] - a[2]);
        if up < 0.0 {
            t.swap(1, 2);
        }
    }
    patch.vertices = verts;
    patch.triangles = tris;
}

// --- mitres -------------------------------------------------------------------

/// Mitred strip ends at a way split.
#[derive(Debug, Clone, PartialEq)]
pub struct Mitre {
    pub vertex: u32,
    /// Per edge: (edge, its `from` end is here, the end row in the strip's
    /// orientation [left, right]).
    pub ends: [(usize, bool, [[f64; 3]; 2]); 2],
    /// How far the inner corner moves back along each strip (m): strip
    /// points nearer the end than this (+ 0.3 m) are dropped.
    pub shift_m: f64,
}

fn build_mitre(g: &RoadGraph, adj: &[usize], v: usize) -> Option<VertexRule> {
    let c = g.vertices[v];
    let (e1, e2) = (&g.edges[adj[0]], &g.edges[adj[1]]);
    if adj[0] == adj[1] || e1.profile.is_empty() || e2.profile.is_empty() {
        return None;
    }
    let out_dir = |e: &Edge| {
        let at_from = e.from.0 as usize == v;
        let pts = &e.points;
        let d = if at_from {
            sub(pts[1], pts[0])
        } else {
            sub(pts[pts.len() - 2], pts[pts.len() - 1])
        };
        (at_from, unit(d))
    };
    let (f1, d1) = out_dir(e1);
    let (f2, d2) = out_dir(e2);
    let bend = std::f64::consts::PI - dot(d1, d2).clamp(-1.0, 1.0).acos();
    if (e1.width_m - e2.width_m).abs() > WIDTH_CHANGE_M || bend > MITRE_MAX_BEND_DEG.to_radians() {
        return build_patch(g, adj, v).map(VertexRule::Patch);
    }
    if bend < MITRE_MIN_BEND_DEG.to_radians() {
        return None;
    }
    // Travel e1 → v → e2.
    let (t1, t2) = (mul(d1, -1.0), d2);
    let m = unit(add(side_pos(t1), side_pos(t2)));
    let k = 1.0 / (bend / 2.0).cos();
    let half = e1.width_m.min(e2.width_m) / 2.0;
    let hv1 = height(e1, if f1 { 0.0 } else { e1.length_m() });
    let hv2 = height(e2, if f2 { 0.0 } else { e2.length_m() });
    let hv = 0.5 * (hv1 + hv2);
    // Travel grades at the vertex (rise per metre of travel).
    let g1 = -grade(
        e1,
        if f1 { 0.0 } else { e1.length_m() },
        if f1 { 1.0 } else { -1.0 },
    );
    let g2 = grade(
        e2,
        if f2 { 0.0 } else { e2.length_m() },
        if f2 { 1.0 } else { -1.0 },
    );
    let corner = |sign: f64| {
        let p = add(c, mul(m, sign * half * k));
        let along1 = dot(sub(p, c), t1);
        let along2 = dot(sub(p, c), t2);
        at_y(p, hv + 0.5 * (g1 * along1 + g2 * along2))
    };
    let (minus, plus) = (corner(-1.0), corner(1.0));
    // Strip orientation: e1's strip runs along t1 if it arrives (to == v).
    let row1 = if !f1 { [minus, plus] } else { [plus, minus] };
    let row2 = if f2 { [minus, plus] } else { [plus, minus] };
    Some(VertexRule::Mitre(Mitre {
        vertex: v as u32,
        ends: [(adj[0], f1, row1), (adj[1], f2, row2)],
        shift_m: half * (bend / 2.0).tan(),
    }))
}

// --- the set ------------------------------------------------------------------

/// What happens at a vertex.
#[derive(Debug, Clone, PartialEq)]
pub enum VertexRule {
    Patch(Patch),
    Mitre(Mitre),
}

/// How a strip's end is shaped.
#[derive(Debug, Clone, Copy)]
pub enum EndRule<'a> {
    Patch(&'a Patch, usize),
    Mitre(&'a Mitre, usize),
}

/// The junctions of a road graph, computed per vertex when first asked.
#[derive(Debug)]
pub struct JunctionSet {
    adjacency: Vec<Vec<usize>>,
    rules: Vec<OnceLock<Option<Arc<VertexRule>>>>,
}

impl Clone for JunctionSet {
    fn clone(&self) -> Self {
        Self {
            adjacency: self.adjacency.clone(),
            rules: self.rules.clone(),
        }
    }
}

impl JunctionSet {
    pub fn new(g: &RoadGraph) -> Self {
        Self {
            adjacency: g.adjacency(),
            rules: (0..g.vertices.len()).map(|_| OnceLock::new()).collect(),
        }
    }

    /// The rule at vertex `v` of `g` (the graph this set was made for).
    pub fn rule(&self, g: &RoadGraph, v: usize) -> Option<&VertexRule> {
        let slot = self.rules.get(v)?;
        slot.get_or_init(|| {
            let adj = &self.adjacency[v];
            if adj.len() < 2
                || adj.len() > MAX_ARMS
                || g.boundary_vertices.contains(&VertexId(v as u32))
                || adj.iter().any(|&e| {
                    let e = &g.edges[e];
                    e.bridge || e.tunnel || e.from == e.to
                })
            {
                return None;
            }
            let rule = if adj.len() == 2 {
                build_mitre(g, adj, v)
            } else {
                build_patch(g, adj, v).map(VertexRule::Patch)
            };
            rule.map(Arc::new)
        })
        .as_deref()
    }

    pub fn patch(&self, g: &RoadGraph, v: usize) -> Option<&Patch> {
        match self.rule(g, v)? {
            VertexRule::Patch(p) => Some(p),
            VertexRule::Mitre(_) => None,
        }
    }

    /// The edges at vertex `v`.
    pub fn edges_at(&self, v: usize) -> &[usize] {
        &self.adjacency[v]
    }

    /// The rules of edge `ei`'s two ends ([from, to]).
    pub fn ends(&self, g: &RoadGraph, ei: usize) -> [Option<EndRule<'_>>; 2] {
        let e = &g.edges[ei];
        let at = |v: usize, from_end: bool| -> Option<EndRule<'_>> {
            match self.rule(g, v)? {
                VertexRule::Patch(p) => p
                    .arms
                    .iter()
                    .position(|a| a.edge == ei && a.at_from == from_end)
                    .map(|i| EndRule::Patch(p, i)),
                VertexRule::Mitre(m) => m
                    .ends
                    .iter()
                    .position(|x| x.0 == ei && x.1 == from_end)
                    .map(|i| EndRule::Mitre(m, i)),
            }
        };
        [at(e.from.0 as usize, true), at(e.to.0 as usize, false)]
    }

    /// The blended row of a side arm `s` metres from the vertex (strip
    /// orientation [left, right]); `s` in [mouth, mouth + blend].
    pub fn blend_row(g: &RoadGraph, p: &Patch, arm: usize, s: f64) -> [[f64; 3]; 2] {
        let a = &p.arms[arm];
        let e = &g.edges[a.edge];
        let cum = cumulative(&e.points);
        let (c, fwd) = frame_at(&e.points, &cum, a.s_edge(s));
        let right = side_pos(fwd);
        let l = a.blend_m.max(1e-9);
        let tt = ((s - a.mouth_m) / l).clamp(0.0, 1.0);
        let end = a.s_edge(a.mouth_m + a.blend_m);
        let p1 = height(e, end);
        let m1 = grade(e, end, if a.at_from { 1.0 } else { -1.0 }) * l;
        let corner = |k: usize| {
            let q = a.row[k];
            let d = 0.05;
            let f = |x: P2| p.main_surface(x);
            let g0 = (f(add(xz(q), mul(a.t, d))) - f(sub(xz(q), mul(a.t, d)))) / (2.0 * d);
            let h = hermite(tt, q[1], g0 * l, p1, m1);
            let side = if k == 0 { -a.half } else { a.half };
            at_y(add(c, mul(right, side)), h)
        };
        [corner(0), corner(1)]
    }

    /// The rows along a side arm's blend: distances from the vertex.
    pub fn blend_steps(a: &Arm) -> Vec<f64> {
        if a.blend_m <= 0.0 {
            return Vec::new();
        }
        let k = (a.blend_m / BLEND_ROW_M).ceil().max(1.0) as usize;
        (0..=k)
            .map(|i| a.mouth_m + a.blend_m * i as f64 / k as f64)
            .collect()
    }

    /// The drawn junction surface at `p` on edge `ei`, `s` metres from its
    /// `from` vertex: inside a patch its mesh, along a blended side arm its
    /// rows; `None` elsewhere (the road's own profile applies).
    pub fn drawn_height(&self, g: &RoadGraph, ei: usize, s: f64, p: P2) -> Option<f64> {
        let ends = self.ends(g, ei);
        let len = g.edges[ei].length_m();
        for (k, end) in ends.iter().enumerate() {
            let Some(EndRule::Patch(patch, ai)) = end else {
                continue;
            };
            let a = &patch.arms[*ai];
            let m = if k == 0 { s } else { len - s };
            if m <= a.mouth_m + 1e-9 {
                return patch.height_at(p).or_else(|| Some(patch.field(p)));
            }
            if a.blend_m > 0.0 && m <= a.mouth_m + a.blend_m {
                let steps = Self::blend_steps(a);
                let i = steps.partition_point(|&x| x <= m).clamp(1, steps.len() - 1);
                let r0 = Self::blend_row(g, patch, *ai, steps[i - 1]);
                let r1 = Self::blend_row(g, patch, *ai, steps[i]);
                // As the strip draws its quads: (l0, r0, l1), (r0, r1, l1).
                for tri in [[r0[0], r0[1], r1[0]], [r0[1], r1[1], r1[0]]] {
                    if let Some(h) = tri_height(tri, p, 1e-6) {
                        return Some(h);
                    }
                }
                // Beside the strip (a rider off the edge): the row line.
                let w = ((m - steps[i - 1]) / (steps[i] - steps[i - 1]).max(1e-9)).clamp(0.0, 1.0);
                let mid0 = lerp3(r0[0], r0[1], 0.5);
                let mid1 = lerp3(r1[0], r1[1], 0.5);
                return Some(mid0[1] + (mid1[1] - mid0[1]) * w);
            }
        }
        None
    }

    /// The drawn surface of the patch at vertex `v` at `p`, if `p` lies in
    /// it (rounded path corners through a junction).
    pub fn patch_height(&self, g: &RoadGraph, v: usize, p: P2) -> Option<f64> {
        self.patch(g, v)?.height_at(p)
    }

    /// Where a patch and a side road's blend lie below the arm's own profile
    /// (the ground the world builder settled under the road): every metre
    /// from the vertex to the end of the blend, the centre point and how
    /// far the drawn surface lies below the profile there (only where it
    /// does, by more than 1 cm). The renderer lowers the terrain by as much
    /// (J1; J2 settles it in the package).
    pub fn sunk(g: &RoadGraph, p: &Patch, arm: usize) -> Vec<([f64; 2], f64)> {
        let a = &p.arms[arm];
        let e = &g.edges[a.edge];
        let cum = cumulative(&e.points);
        let end = a.mouth_m + a.blend_m;
        let n = end.ceil().max(1.0) as usize;
        let mut out = Vec::new();
        for k in 0..=n {
            let s = end * k as f64 / n as f64;
            let (c, _) = frame_at(&e.points, &cum, a.s_edge(s));
            let drawn = if s <= a.mouth_m {
                p.height_at(c).unwrap_or_else(|| p.field(c))
            } else {
                let r = Self::blend_row(g, p, arm, s);
                0.5 * (r[0][1] + r[1][1])
            };
            let delta = drawn - height(e, a.s_edge(s));
            if delta < -0.01 {
                out.push((c, delta));
            }
        }
        out
    }

    /// How far a junction reaches into edge `ei` from each end (m; 0: not
    /// at all): the mouth plus a side arm's blend.
    pub fn reach(&self, g: &RoadGraph, ei: usize) -> [f64; 2] {
        self.ends(g, ei).map(|e| match e {
            Some(EndRule::Patch(p, i)) => p.arms[i].mouth_m + p.arms[i].blend_m,
            _ => 0.0,
        })
    }
}

/// A road strip shaped for its junctions (`ocw-render` draws it as it is).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shaped {
    /// Centre points (x, y, z).
    pub points: Vec<[f32; 3]>,
    /// Per point the carriageway's edge points [left, right].
    pub rows: Vec<[[f32; 3]; 2]>,
    /// 3D length cut off the start and the end of the strip (m): the dress's
    /// `start_m` and the crossings move by it.
    pub cut_m: [f32; 2],
    /// The ends that now meet a junction surface (no junction-zone taper of
    /// the kerbs there).
    pub junction_end: [bool; 2],
}

/// The strip's points: distances along the edge from its `from` vertex.
fn strip_distances(e: &Edge, cum: &[f64], pts: &[[f32; 3]]) -> Vec<f64> {
    let s0 = project(e, cum, pts[0]);
    let mut out = Vec::with_capacity(pts.len());
    let mut s = s0;
    out.push(s);
    for w in pts.windows(2) {
        s += f64::from((w[1][0] - w[0][0]).hypot(w[1][2] - w[0][2]));
        out.push(s);
    }
    out
}

/// Whether a strip of edge `e` runs against the edge's direction (from its
/// `to` end to its `from` end; some packages store strips so).
pub fn strip_reversed(e: &Edge, pts: &[[f32; 3]]) -> bool {
    if pts.len() < 2 || e.points.len() < 2 {
        return false;
    }
    let cum = cumulative(&e.points);
    project(e, &cum, pts[pts.len() - 1]) < project(e, &cum, pts[0])
}

/// Distance along the edge of the point nearest to `p`.
fn project(e: &Edge, cum: &[f64], p: [f32; 3]) -> f64 {
    let p0 = [f64::from(p[0]), f64::from(p[2])];
    // Projection onto the edge, extrapolated past its ends (the strips reach
    // a little beyond a region-border cut).
    let mut best = (f64::INFINITY, 0.0);
    let n = e.points.len();
    for k in 0..n - 1 {
        let (a, b) = (e.points[k], e.points[k + 1]);
        let ab = sub(b, a);
        let l2 = dot(ab, ab);
        if l2 <= 0.0 {
            continue;
        }
        let mut t = dot(sub(p0, a), ab) / l2;
        if k > 0 {
            t = t.max(0.0);
        }
        if k + 2 < n {
            t = t.min(1.0);
        }
        let q = add(a, mul(ab, t));
        let d = norm(sub(q, p0));
        if d < best.0 - 1e-9 {
            best = (d, cum[k] + t * l2.sqrt());
        }
    }
    best.1
}

fn f32p(p: [f64; 3]) -> [f32; 3] {
    p.map(|x| x as f32)
}

/// Rows of plain points: flat across, by the points' central differences
/// (as `road_mesh` draws an unshaped strip).
fn plain_row(pts: &[[f32; 3]], i: usize, half: f32) -> [[f32; 3]; 2] {
    let n = pts.len();
    let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(n - 1)]);
    let d = [b[0] - a[0], b[2] - a[2]];
    let l = d[0].hypot(d[1]).max(1e-9);
    let r = [-d[1] / l, d[0] / l];
    let p = pts[i];
    [
        [p[0] - r[0] * half, p[1], p[2] - r[1] * half],
        [p[0] + r[0] * half, p[1], p[2] + r[1] * half],
    ]
}

impl JunctionSet {
    /// Shapes a strip of edge `ei` (its points as the package has them):
    /// trimmed at the mouths, re-heighted along blended side arms, mitred
    /// at way splits. `None`: the strip is unchanged; an empty result: it
    /// lies inside a junction surface.
    pub fn shape_strip(
        &self,
        g: &RoadGraph,
        ei: usize,
        pts: &[[f32; 3]],
        half: f32,
    ) -> Option<Shaped> {
        if pts.len() < 2 {
            return None;
        }
        if !strip_reversed(&g.edges[ei], pts) {
            return self.shape_forward(g, ei, pts, half);
        }
        // A strip against its edge: shaped along the edge, then turned back.
        let rev: Vec<[f32; 3]> = pts.iter().rev().copied().collect();
        let mut sh = self.shape_forward(g, ei, &rev, half)?;
        sh.points.reverse();
        sh.rows.reverse();
        for r in &mut sh.rows {
            r.swap(0, 1);
        }
        sh.cut_m.swap(0, 1);
        sh.junction_end.swap(0, 1);
        Some(sh)
    }

    fn shape_forward(
        &self,
        g: &RoadGraph,
        ei: usize,
        pts: &[[f32; 3]],
        half: f32,
    ) -> Option<Shaped> {
        let ends = self.ends(g, ei);
        if ends.iter().all(Option::is_none) {
            return None;
        }
        let e = &g.edges[ei];
        let len = e.length_m();
        let cum = cumulative(&e.points);
        let s = strip_distances(e, &cum, pts);
        let (s_first, s_last) = (s[0], s[s.len() - 1]);
        // Distance windows (edge metres) the strip keeps: [lo, hi].
        let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
        // Generated rows: (edge s, row) and the ranges they replace.
        let mut generated: Vec<(f64, [[f64; 3]; 2])> = Vec::new();
        let mut replaced: Vec<(f64, f64)> = Vec::new();
        let mut mitres: [Option<[[f64; 3]; 2]>; 2] = [None, None];
        let mut mitre_drop = [0.0f64; 2];
        let mut junction_end = [false; 2];
        for (k, end) in ends.iter().enumerate() {
            match end {
                Some(EndRule::Patch(p, ai)) => {
                    let a = &p.arms[*ai];
                    junction_end[k] = true;
                    let m_edge = a.s_edge(a.mouth_m);
                    if k == 0 {
                        lo = m_edge;
                    } else {
                        hi = m_edge;
                    }
                    generated.push((m_edge, a.row));
                    if a.blend_m > 0.0 {
                        let steps = Self::blend_steps(a);
                        let (b0, b1) = (a.s_edge(steps[0]), a.s_edge(*steps.last().unwrap()));
                        replaced.push((b0.min(b1), b0.max(b1)));
                        for &st in &steps[1..] {
                            generated.push((a.s_edge(st), Self::blend_row(g, p, *ai, st)));
                        }
                        // The strip piece may start or end inside the blend.
                        for se in [s_first, s_last] {
                            let m = if k == 0 { se } else { len - se };
                            if m > a.mouth_m + 1e-6 && m < a.mouth_m + a.blend_m - 1e-6 {
                                generated.push((se, Self::blend_row(g, p, *ai, m)));
                            }
                        }
                    }
                }
                Some(EndRule::Mitre(m, i)) => {
                    let at_end = if k == 0 {
                        s_first < 0.01
                    } else {
                        s_last > len - 0.01
                    };
                    if at_end {
                        mitres[k] = Some(m.ends[*i].2);
                        mitre_drop[k] = m.shift_m + 0.3;
                    }
                }
                None => {}
            }
        }
        if s_last <= lo + 1e-6 || s_first >= hi - 1e-6 {
            return Some(Shaped::default());
        }
        // Kept plain points.
        let n = pts.len();
        let mut items: Vec<(f64, [f32; 3], Option<[[f32; 3]; 2]>)> = Vec::new();
        for i in 0..n {
            let si = s[i];
            if si <= lo + 1e-6 || si >= hi - 1e-6 {
                continue;
            }
            if replaced
                .iter()
                .any(|&(a, b)| si >= a - 1e-6 && si <= b + 1e-6)
            {
                continue;
            }
            let is_end = (i == 0 && mitres[0].is_some()) || (i == n - 1 && mitres[1].is_some());
            if !is_end
                && ((mitres[0].is_some() && si < s_first + mitre_drop[0] && i > 0)
                    || (mitres[1].is_some() && si > s_last - mitre_drop[1] && i < n - 1))
                && n > 2
            {
                continue;
            }
            let row = if i == 0 {
                mitres[0].map(|r| r.map(f32p))
            } else if i == n - 1 {
                mitres[1].map(|r| r.map(f32p))
            } else {
                None
            };
            items.push((si, pts[i], row));
        }
        for (sg, row) in generated {
            if sg < lo - 1e-6 || sg > hi + 1e-6 || sg < s_first - 1e-6 || sg > s_last + 1e-6 {
                continue;
            }
            if items
                .iter()
                .any(|it| (it.0 - sg).abs() < 1e-6 && it.2.is_some())
            {
                continue;
            }
            items.retain(|it| (it.0 - sg).abs() >= 1e-6);
            let r = row.map(f32p);
            let c = [
                0.5 * (r[0][0] + r[1][0]),
                0.5 * (r[0][1] + r[1][1]),
                0.5 * (r[0][2] + r[1][2]),
            ];
            items.push((sg, c, Some(r)));
        }
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        if items.len() < 2 {
            return Some(Shaped::default());
        }
        let points: Vec<[f32; 3]> = items.iter().map(|it| it.1).collect();
        let rows: Vec<[[f32; 3]; 2]> = items
            .iter()
            .enumerate()
            .map(|(i, it)| it.2.unwrap_or_else(|| plain_row(&points, i, half)))
            .collect();
        // 3D length cut off each end, along the original points.
        let mut cum3 = vec![0.0f64; n];
        for i in 1..n {
            let (a, b) = (pts[i - 1], pts[i]);
            cum3[i] = cum3[i - 1]
                + f64::from(
                    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt(),
                );
        }
        let at3 = |sq: f64| {
            let k = s.partition_point(|&x| x <= sq).clamp(1, n - 1);
            let seg = (s[k] - s[k - 1]).max(1e-9);
            let t = ((sq - s[k - 1]) / seg).clamp(0.0, 1.0);
            cum3[k - 1] + (cum3[k] - cum3[k - 1]) * t
        };
        let first = items[0].0;
        let last = items[items.len() - 1].0;
        let cut = [at3(first) as f32, (cum3[n - 1] - at3(last)) as f32];
        Some(Shaped {
            points,
            rows,
            cut_m: cut.map(|c| c.max(0.0)),
            junction_end,
        })
    }
}

pub mod fixtures;

#[cfg(test)]
mod tests;
