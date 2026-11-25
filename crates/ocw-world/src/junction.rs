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
//! Vertices that cannot be patched (forks below [`MIN_CORNER_DEG`] or too
//! long for their mouths,
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
pub const MIN_CORNER_DEG: f64 = 4.0;
/// Corners narrower than this (degrees) are a fork (J3): its two arms
/// separate far out, so their mouths may lie up to [`FORK_MAX_MOUTH_M`]
/// from the vertex (J1 left forks below 15° without a patch).
pub const FORK_DEG: f64 = 15.0;
pub const FORK_MAX_MOUTH_M: f64 = 40.0;
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
/// The first interior ring lies this far inside the outline (m), the
/// second 2.5 times as far (J3).
pub const OUTER_RING_M: f64 = 0.4;
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
    /// The arm's own vertex (the patch's, or a cluster member's, J3).
    pub origin: P2,
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
    /// Side arm of a through road whose own surface starts at the main
    /// road's edge (§4.8.8 Q6 (d), J3): metres from the vertex where its
    /// centre line leaves the main road's carriageway (0: not such an arm –
    /// a main arm, an arm of a blended patch, or a side arm whose mouth
    /// lies at the main road's edge, blended as in J2), that point and the
    /// main road's surface there. Its mouth row lies at its own height and
    /// grade (no blend); the corner between the main road's edge and the
    /// mouth slopes from the one to the other ([`Patch::field`]).
    pub edge_m: f64,
    pub edge_p: P2,
    pub edge_h: f64,
    /// Its profile from the vertex to the mouth: (metres from the vertex,
    /// height), the edge's samples and both ends.
    pub corner: Vec<(f64, f64)>,
    /// The cross slope of the arm's surface at its mouth (to its right in
    /// the mouth's frame, `t` turned by [`side_pos`]): an arm of a blended
    /// patch (no through road) takes the plane through all its arms
    /// (§4.8.8 Q7 (c)), a side road with a corner (Q6 (d)) the main road's
    /// slope across it at its edge point (J5) – inside the patch its
    /// surface, and its mouth row, lean so; past the mouth its strip turns
    /// flat across over [`Arm::warp_m`] (§4.8.8 Rule: the corner between
    /// two roads no longer twists from the one's slope to a flat mouth
    /// within a few metres). 0 elsewhere.
    pub cross: f64,
    /// How far past the mouth the strip turns flat across (m; 0: flat at
    /// the mouth).
    pub warp_m: f64,
    /// An arm that runs on past a plain way split (J8): the centre line
    /// from the patch's vertex to [`Arm::origin`] (the split, both
    /// included) along the edges inside the patch before it; empty for an
    /// arm leaving from its vertex.
    pub lead: Vec<P2>,
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

    /// The arm's surface extended past its mouth: along its grade, with its
    /// cross slope ([`Arm::cross`]).
    fn extension(&self, p: P2) -> f64 {
        let along = dot(sub(p, self.centre), self.t);
        self.h + self.grade * along + self.cross * self.across(p)
    }

    /// How far `p` lies to the right of the arm's centre line in its
    /// mouth's frame (m).
    fn across(&self, p: P2) -> f64 {
        dot(sub(p, self.centre), side_pos(self.t))
    }

    /// The cross slope and the warp length for a mouth row leaning by
    /// `cross` (clamped to [`MAX_WARP`]); none where the arm has no room
    /// past its mouth to turn flat.
    fn lean(&mut self, cross: f64) {
        // The room past the mouth before the other end's junction (its
        // mouth at most [`MOUTH_SHARE`] of the edge): the strip turns flat
        // within it.
        let room = (1.0 - MOUTH_SHARE) * self.len - self.mouth_m;
        if room < 1.0 {
            return;
        }
        let cross = cross.clamp(-MAX_WARP, MAX_WARP);
        let rise = cross.abs() * self.half;
        if rise < 0.005 {
            return;
        }
        let l = (WARP_RATE * rise)
            .sqrt()
            .clamp(WARP_MIN_M, WARP_MAX_M)
            .min(room);
        self.cross = cross;
        self.warp_m = l;
        let (c, t) = (self.centre, self.t);
        for k in 0..2 {
            let q = xz(self.row[k]);
            let across = dot(sub(q, c), side_pos(t));
            self.row[k][1] = self.h + cross * across;
        }
    }

    fn mouth_dist(&self, p: P2) -> f64 {
        seg_dist(p, xz(self.row[0]), xz(self.row[1]))
    }

    /// A side arm whose own surface starts at the main road's edge (Q6 (d)).
    pub fn has_corner(&self) -> bool {
        self.edge_m > 0.0
    }

    /// The arm's profile `m` metres from the vertex (between the samples
    /// as the edge interpolates them; past the mouth its extension).
    pub fn corner_profile(&self, m: f64) -> f64 {
        if m >= self.mouth_m || self.corner.len() < 2 {
            return self.h + self.grade * (m - self.mouth_m);
        }
        let c = &self.corner;
        let k = c.partition_point(|q| q.0 <= m).clamp(1, c.len() - 1);
        let ((m0, h0), (m1, h1)) = (c[k - 1], c[k]);
        let t = if m1 > m0 {
            ((m - m0) / (m1 - m0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        h0 + (h1 - h0) * t
    }

    /// The side road's surface in its corner at `p` (flat across, by the
    /// distance along the arm in the mouth's frame): its own profile, plus
    /// what it misses of the main road's surface at the edge point, faded
    /// out towards the mouth (nothing when the profile already starts
    /// there, as the world builder makes it) – and the distance of `p`
    /// from the arm's centre line in the corner or from its mouth row.
    fn corner_node(&self, p: P2) -> (f64, f64) {
        let m = (self.mouth_m + dot(sub(p, self.centre), self.t)).max(self.edge_m);
        let span = (self.mouth_m - self.edge_m).max(1e-6);
        let x = ((m - self.edge_m) / span).clamp(0.0, 1.0);
        let fade = 1.0 - x * x * (3.0 - 2.0 * x);
        let miss = self.edge_h - self.corner_profile(self.edge_m);
        let q = self.corner_profile(m) + miss * fade + self.cross * self.across(p);
        let rho = seg_dist(p, self.edge_p, self.centre).min(self.mouth_dist(p));
        (rho, q)
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
    /// The vertex, or a cluster's centroid (J3).
    pub centre: [f64; 3],
    /// The vertices this patch covers (J3 clusters: several, joined by the
    /// short edges in `inner`; else only `vertex`).
    pub members: Vec<u32>,
    pub inner: Vec<usize>,
    /// The members that are plain way splits its arms run on past (J8; the
    /// edges before them in `inner`): one junction, drawn as at its vertex.
    pub run_on: Vec<u32>,
    inner_nodes: Vec<InnerNode>,
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
    /// The triangles of each sector between outline points `k` and `k + 1`
    /// (the star's quads, cut where they cross the through road's
    /// carriageway edge, §4.8.8 Q7).
    sectors: Vec<Vec<u32>>,
    /// How far each outline point is lifted (m, negative: lowered) from
    /// the height field so that the sides between the mouths run smoothly
    /// (J5, §4.8.8 Rule); the surface follows it, fading out inward over
    /// [`SIDE_LIFT_FADE_M`]. Empty before the outline is known.
    lift: Vec<f64>,
    /// An outline that is not star-shaped from the centre (J6): its
    /// triangles by 1 m cells (`plane_mesh`); `None` for the star mesh.
    cells: Option<plane_mesh::CellIndex>,
    /// Star mesh: ring j point k is `vertices[j * outline.len() + k]`
    /// (J6's mesh: the outline points first, then the inner points).
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub min: P2,
    pub max: P2,
}

/// An edge inside a cluster patch (J3): its own profile is a node of the
/// height field along its centre line.
#[derive(Debug, Clone, PartialEq)]
struct InnerNode {
    points: Vec<P2>,
    cum: Vec<f64>,
    profile: Vec<f64>,
    len: f64,
    half: f64,
    /// Of the through road's class (part of its carriageway).
    main: bool,
}

impl InnerNode {
    fn of(g: &RoadGraph, ei: usize) -> Self {
        let e = &g.edges[ei];
        Self {
            points: e.points.clone(),
            cum: cumulative(&e.points),
            profile: e.profile.clone(),
            len: e.length_m(),
            half: e.width_m / 2.0,
            main: false,
        }
    }

    /// The distance of `p` from the centre line and the profile's height
    /// at its nearest point.
    fn node(&self, p: P2) -> (f64, f64) {
        let mut best = (f64::INFINITY, 0.0);
        for (k, w) in self.points.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let ab = sub(b, a);
            let l2 = dot(ab, ab);
            let t = if l2 > 0.0 {
                (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = norm(sub(add(a, mul(ab, t)), p));
            if d < best.0 {
                best = (d, self.cum[k] + t * l2.sqrt());
            }
        }
        let s = best.1.clamp(0.0, self.len);
        let n = self.profile.len();
        let h = if n < 2 {
            self.profile.first().copied().unwrap_or(0.0)
        } else {
            let i = ((s / PROFILE_STEP_M).floor() as usize).min(n - 2);
            let (s0, s1) = (
                (i as f64 * PROFILE_STEP_M).min(self.len),
                ((i + 1) as f64 * PROFILE_STEP_M).min(self.len),
            );
            let t = if s1 > s0 {
                ((s - s0) / (s1 - s0)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            self.profile[i] + (self.profile[i + 1] - self.profile[i]) * t
        };
        (best.0, h)
    }
}

/// Where a patch lies: its centre, the vertices it covers and the edges
/// inside it (a single vertex, or a J3 cluster).
struct Site {
    centre: P2,
    members: Vec<u32>,
    inner: Vec<usize>,
    /// The members that are plain way splits its arms run on past (J8):
    /// not junctions of their own, not counted in the centre.
    run_on: Vec<u32>,
    /// The mouths move out for their corners along the real edge lines
    /// (J8; not for a junction's patch alone, which decides the clusters
    /// and the way splits run on past as J7 did).
    grow: bool,
    /// A cluster whose outline may be any simple polygon, not only a
    /// star-shaped one (J9: where the star-shaped one cannot be made).
    free: bool,
}

impl Site {
    fn single(g: &RoadGraph, v: usize) -> Self {
        Self {
            centre: g.vertices[v],
            members: vec![v as u32],
            inner: Vec::new(),
            run_on: Vec::new(),
            grow: true,
            free: false,
        }
    }

    /// Several junctions (J3), not one with its arms run on (J8).
    fn cluster(&self) -> bool {
        self.members.len() > self.run_on.len() + 1
    }
}

impl Patch {
    /// Whether this patch belongs to vertex `v` (a cluster's to its first
    /// member only): iterate patches once, draw them once.
    pub fn is_at(&self, v: usize) -> bool {
        self.vertex as usize == v
    }

    /// A cluster of several junctions (J3); not one junction whose arms
    /// run on past way splits (J8).
    pub fn is_cluster(&self) -> bool {
        self.members.len() > self.run_on.len() + 1
    }

    fn n(&self) -> usize {
        self.outline.len()
    }

    /// The patch's own height field at `p` (exact, not the mesh). With a
    /// through road (§4.8.8 Q1) its own surface on its carriageway; beyond
    /// its edges (§4.8.8 Q6 (d), J3) an inverse-square blend of the main
    /// road's surface (by the distance from its carriageway) and each side
    /// road's own surface in its corner (by the distance from its centre
    /// line there or from its mouth): the corners slope from the main
    /// road's edge to the side roads, C1 at the edge and at the mouths.
    pub fn field(&self, p: P2) -> f64 {
        let Some([a, b]) = self.through else {
            return self.blended(p);
        };
        let main = shepard([&self.arms[a], &self.arms[b]].into_iter(), p);
        if !self.arms.iter().any(Arm::has_corner) {
            return main;
        }
        let e = self.main_excess(p);
        if e <= 0.0 {
            return main;
        }
        // The main surface's nodes: its carriageway, and the mouths on it
        // (its own arms', the side arms blended as in J2).
        let mut dp = e;
        for arm in &self.arms {
            if !arm.has_corner() {
                dp = dp.min(arm.mouth_dist(p));
            }
        }
        if dp < 1e-9 {
            return main;
        }
        let (mut sw, mut sh) = (1.0 / (dp * dp), main / (dp * dp));
        for arm in self.arms.iter().filter(|a| a.has_corner()) {
            let (rho, q) = arm.corner_node(p);
            if rho < 1e-9 {
                return q;
            }
            let w = 1.0 / (rho * rho);
            sw += w;
            sh += w * q;
        }
        sh / sw
    }

    /// How far `p` lies outside the through road's carriageway (m; 0 on
    /// it): beyond the edge lines of its two arms (the straight lines the
    /// outline follows), from their vertex to past their mouths, and in a
    /// cluster beside the edges inside of its class.
    pub fn main_excess(&self, p: P2) -> f64 {
        let Some([a, b]) = self.through else {
            return 0.0;
        };
        let mut e = f64::INFINITY;
        for k in [a, b] {
            let arm = &self.arms[k];
            let end = add(arm.origin, mul(arm.dir, arm.mouth_m + 1.0));
            e = e.min(seg_dist(p, arm.origin, end) - arm.half);
        }
        for n in self.inner_nodes.iter().filter(|n| n.main) {
            e = e.min(n.node(p).0 - n.half);
        }
        e.max(0.0)
    }

    /// The through road's drawn surface at `p` if `p` lies on its
    /// carriageway near the patch, or within `margin` of it: on the patch
    /// its own surface, beside its two arms (their edges' centre lines, up
    /// to `reach` metres past their mouths) their profile; `None`
    /// elsewhere.
    pub fn main_road_height(&self, g: &RoadGraph, p: P2, reach: f64, margin: f64) -> Option<f64> {
        let [a, b] = self.through?;
        if self.main_excess(p) <= margin {
            return self.main_own(p);
        }
        let mut best: Option<f64> = None;
        for k in [a, b] {
            let arm = &self.arms[k];
            let e = &g.edges[arm.edge];
            let mut walked = 0.0;
            for w in e.points.windows(2) {
                let (q0, q1) = (w[0], w[1]);
                let l = norm(sub(q1, q0));
                if l > 1e-9 {
                    let t = (dot(sub(p, q0), sub(q1, q0)) / (l * l)).clamp(0.0, 1.0);
                    let s = walked + t * l;
                    let m = if arm.at_from { s } else { arm.len - s };
                    if m <= arm.mouth_m + reach
                        && norm(sub(add(q0, mul(sub(q1, q0), t)), p)) <= arm.half + margin
                    {
                        let h = height(e, s);
                        best = Some(best.map_or(h, |x: f64| x.max(h)));
                    }
                }
                walked += l;
            }
        }
        best
    }

    /// A blended patch's field (no through road): the arms' extensions
    /// and, in a cluster, the edges inside along their centre lines, by
    /// inverse-square distance.
    fn blended(&self, p: P2) -> f64 {
        let (mut sw, mut sh) = (0.0, 0.0);
        for a in &self.arms {
            let d = a.mouth_dist(p);
            if d < 1e-9 {
                return a.extension(p);
            }
            let w = 1.0 / (d * d);
            sw += w;
            sh += w * a.extension(p);
        }
        for n in &self.inner_nodes {
            let (d, h) = n.node(p);
            if d < 1e-9 {
                return h;
            }
            let w = 1.0 / (d * d);
            sw += w;
            sh += w * h;
        }
        if sw > 0.0 { sh / sw } else { 0.0 }
    }

    /// The main road's surface (the through road's; else the field).
    pub fn main_surface(&self, p: P2) -> f64 {
        self.field(p)
    }

    /// The surface the patch is drawn from: its height field with the
    /// sides' lift ([`Patch::field`] near the outline smoothed along it,
    /// J5).
    pub fn surface(&self, p: P2) -> f64 {
        self.field(p) + self.lift_at(p)
    }

    /// The sides' lift at `p`: between the two outline points of its
    /// sector, fading out inward over [`SIDE_LIFT_FADE_M`]; never on the
    /// through road's carriageway.
    fn lift_at(&self, p: P2) -> f64 {
        let n = self.n();
        if self.lift.len() != n || n < 3 || self.lift.iter().all(|&l| l == 0.0) {
            return 0.0;
        }
        if self.angles.len() != n {
            // Not star-shaped (J6): from the nearest piece of the outline.
            let pts: Vec<P2> = self.outline.iter().map(|o| xz(o.p)).collect();
            let (d, k, lam) = plane_mesh::nearest_piece(&pts, p);
            let k1 = (k + 1) % n;
            return self.lift_on(self.lift[k] + (self.lift[k1] - self.lift[k]) * lam, d, p);
        }
        let c = xz(self.centre);
        let v = sub(p, c);
        if norm(v) < 1e-9 {
            return 0.0;
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
        let d = (norm(q) - norm(v)).max(0.0);
        self.lift_on(self.lift[k] + (self.lift[k1] - self.lift[k]) * lam, d, p)
    }

    /// A lift `l` of the outline `d` metres in from it at `p`.
    fn lift_on(&self, l: f64, d: f64, p: P2) -> f64 {
        if l == 0.0 {
            return 0.0;
        }
        let x = (d / SIDE_LIFT_FADE_M).clamp(0.0, 1.0);
        let mut f = 1.0 - x * x * (3.0 - 2.0 * x);
        if self.through.is_some() {
            f *= (self.main_excess(p) / SIDE_LIFT_FADE_M).clamp(0.0, 1.0);
        }
        l * f
    }

    /// The through road's own surface at `p` (its two arms' extensions,
    /// without the side roads' corners); `None` without a through road.
    pub fn main_own(&self, p: P2) -> Option<f64> {
        let [a, b] = self.through?;
        Some(shepard([&self.arms[a], &self.arms[b]].into_iter(), p))
    }

    /// Whether the outline is star-shaped from the centre (the star mesh);
    /// J6 meshes other simple outlines by `plane_mesh`.
    pub fn is_star(&self) -> bool {
        self.cells.is_none()
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
        if let Some(cells) = &self.cells {
            if p[0] < self.min[0] - 0.01
                || p[1] < self.min[1] - 0.01
                || p[0] > self.max[0] + 0.01
                || p[1] > self.max[1] + 0.01
            {
                return None;
            }
            for tol in [1e-7, 1e-5] {
                for &ti in cells.at(p) {
                    let tri = self.triangles[ti as usize].map(|i| self.vertices[i as usize]);
                    if let Some(h) = tri_height(tri, p, tol) {
                        return Some(h);
                    }
                }
            }
            return None;
        }
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
        // The sector's triangles, then its neighbours' (on a spoke).
        let ns = self.sectors.len();
        for kk in [k, (k + ns - 1) % ns.max(1), (k + 1) % ns.max(1)] {
            for &ti in self.sectors.get(kk).into_iter().flatten() {
                let tri = self.triangles[ti as usize].map(|i| self.vertices[i as usize]);
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
    try_patch(g, adj, v).ok()
}

/// An arm before its mouth is known.
#[derive(Clone)]
struct Raw {
    edge: usize,
    at_from: bool,
    /// The arm's own vertex (a cluster's arms leave from different ones)
    /// and a point 8 m out along it (its polar angle orders the arms).
    origin: P2,
    refp: P2,
    dir: P2,
    half: f64,
    len: f64,
    class: RoadClass,
    cum: Vec<f64>,
    /// [`Arm::lead`] (J8).
    lead: Vec<P2>,
    /// How far out its mouth may lie where the edge's far end leaves room
    /// (J9; `None`: its share, [`MOUTH_SHARE`]).
    room: Option<f64>,
}

/// The arm of edge `ei` at vertex `v` (at `c`).
fn raw_arm(g: &RoadGraph, c: P2, v: usize, ei: usize) -> Result<Raw, &'static str> {
    let e = &g.edges[ei];
    let len = e.length_m();
    if len < 1.0 || e.profile.is_empty() {
        return Err("a short edge or no profile");
    }
    let at_from = e.from.0 as usize == v;
    let cum = cumulative(&e.points);
    let d = 8.0f64.min(MOUTH_SHARE * len);
    let s = if at_from { d } else { len - d };
    let (p, _) = frame_at(&e.points, &cum, s);
    Ok(Raw {
        edge: ei,
        at_from,
        origin: c,
        refp: p,
        dir: unit(sub(p, c)),
        half: e.width_m / 2.0,
        len,
        class: e.class,
        cum,
        lead: Vec::new(),
        room: None,
    })
}

/// The arm of edge `oi` leaving the plain way split `w` that edge `ei`
/// leads to from vertex `v` (J8): it runs on past the split, `ei` inside
/// the patch.
fn raw_run_on(
    g: &RoadGraph,
    v: usize,
    ei: usize,
    w: usize,
    oi: usize,
) -> Result<Raw, &'static str> {
    let mut r = raw_arm(g, g.vertices[w], w, oi)?;
    let e = &g.edges[ei];
    let mut lead = e.points.clone();
    if e.from.0 as usize != v {
        lead.reverse();
    }
    r.lead = lead;
    Ok(r)
}

/// The arm with its mouth `m` metres from the vertex; `along` (the
/// strip's along coordinate there) when it is known (a stored outline,
/// section `JNCT`), else computed.
fn make_arm(g: &RoadGraph, r: &Raw, m: f64, along: Option<f64>) -> Arm {
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
    Arm {
        edge: r.edge,
        at_from: r.at_from,
        class: r.class,
        half: r.half,
        len: r.len,
        dir: r.dir,
        origin: r.origin,
        mouth_m: m,
        centre: cp,
        t,
        row,
        h,
        grade: grade(e, s, if r.at_from { 1.0 } else { -1.0 }),
        blend_m: 0.0,
        along: along.unwrap_or_else(|| along_at(e, s)),
        edge_m: 0.0,
        edge_p: cp,
        edge_h: h,
        corner: Vec::new(),
        cross: 0.0,
        warp_m: 0.0,
        lead: r.lead.clone(),
    }
}

/// The patch of `arms` (by polar angle) at vertex `v`: the through road
/// (§4.8.8 Q1) and its side arms' mouth rows on the main surface with
/// their blends; no outline yet.
fn assemble(g: &RoadGraph, site: &Site, arms: Vec<Arm>) -> Result<Patch, &'static str> {
    let c = site.centre;
    let v = site.members[0] as usize;
    let n = arms.len();
    // Through road: two arms of the highest class, nearly straight; only
    // one such pair.
    let best = arms.iter().map(|a| a.class).min().ok_or("no arms")?;
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
    // A blended patch's plane (Q7 (c)); clusters keep their surface.
    let mut arms = arms;
    if through.is_none() && !site.cluster() {
        blend_plane(&mut arms);
    }
    // The height at the vertex: of the first arm's edge there (an arm run
    // on past a way split leaves from the split, J8: the first that leaves
    // from the vertex, or the edge inside the patch at it).
    let vh = match arms.iter().find(|a| a.lead.is_empty()) {
        Some(a) if site.run_on.is_empty() || a.origin == c => {
            height(&g.edges[a.edge], if a.at_from { 0.0 } else { a.len })
        }
        _ => {
            let a = &arms[0];
            let at_v = site.inner.iter().find_map(|&ei| {
                let e = &g.edges[ei];
                if e.from.0 as usize == v {
                    Some(height(e, 0.0))
                } else if e.to.0 as usize == v {
                    Some(height(e, e.length_m()))
                } else {
                    None
                }
            });
            at_v.unwrap_or_else(|| height(&g.edges[a.edge], if a.at_from { 0.0 } else { a.len }))
        }
    };
    // An edge run on past (J8) is of the through road's carriageway when
    // its arm is one of the through road's.
    let leads_to = |ei: usize, k: usize| {
        let e = &g.edges[ei];
        !arms[k].lead.is_empty()
            && [e.from, e.to]
                .iter()
                .any(|w| g.vertices[w.0 as usize] == arms[k].origin)
    };
    let inner_nodes = site
        .inner
        .iter()
        .map(|&ei| {
            let mut node = InnerNode::of(g, ei);
            node.main = match through {
                Some([a, b]) if !site.run_on.is_empty() => leads_to(ei, a) || leads_to(ei, b),
                _ => through.is_some() && g.edges[ei].class == best,
            };
            node
        })
        .collect();
    let mut patch = Patch {
        vertex: v as u32,
        centre: at_y(c, vh),
        members: site.members.clone(),
        inner: site.inner.clone(),
        run_on: site.run_on.clone(),
        inner_nodes,
        arms,
        through,
        outline: Vec::new(),
        angles: Vec::new(),
        chains: Vec::new(),
        sectors: Vec::new(),
        lift: Vec::new(),
        cells: None,
        vertices: Vec::new(),
        triangles: Vec::new(),
        min: c,
        max: c,
    };
    // Side arms of a through road (§4.8.8 Q6 (d), J3): their own surface
    // starts where their centre line leaves the main road's carriageway;
    // the mouth row lies at their own height (no blend), the corner slopes
    // between. A side arm whose mouth lies at the main road's edge (no room
    // for a corner) keeps J2's rule: its mouth row on the main surface, a
    // blend beyond.
    if let Some([a, b]) = through {
        for i in 0..n {
            if i == a || i == b {
                continue;
            }
            // In a cluster a side arm may leave from a vertex off the main
            // road's carriageway: its corner follows its own profile from
            // there (nothing missed at the edge point).
            if patch.main_excess(patch.arms[i].origin) > 0.0 {
                let m = 1e-3;
                let corner = corner_samples(g, &patch.arms[i], m);
                let arm = &mut patch.arms[i];
                arm.edge_m = m;
                arm.edge_p = arm.origin;
                arm.corner = corner;
                arm.edge_h = arm.corner_profile(m);
                continue;
            }
            if let Some((m, pt)) = edge_exit(g, &patch, i)
                && m < patch.arms[i].mouth_m - CORNER_MIN_M
            {
                let main = shepard([&patch.arms[a], &patch.arms[b]].into_iter(), pt);
                let corner = corner_samples(g, &patch.arms[i], m);
                // The main road's slope across the side road at its edge
                // point: the side road's surface leans so in its corner and
                // turns flat past its mouth (J5).
                let d = 0.05;
                let f = |x: P2| shepard([&patch.arms[a], &patch.arms[b]].into_iter(), x);
                let grad = [
                    (f(add(pt, [d, 0.0])) - f(sub(pt, [d, 0.0]))) / (2.0 * d),
                    (f(add(pt, [0.0, d])) - f(sub(pt, [0.0, d]))) / (2.0 * d),
                ];
                let arm = &mut patch.arms[i];
                arm.edge_m = m;
                arm.edge_p = pt;
                arm.edge_h = main;
                arm.corner = corner;
                let cross = dot(grad, side_pos(arm.t));
                arm.lean(cross);
                continue;
            }
            let row = patch.arms[i].row.map(|q| {
                let main = shepard([&patch.arms[a], &patch.arms[b]].into_iter(), xz(q));
                at_y(xz(q), main)
            });
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
        // §4.8.8 Q7: a side road's mouth row never stands above the main
        // road's carriageway (a path drawn along the main road, its mouth
        // corner on it): such a corner lies on the main road's surface.
        for i in 0..n {
            if i == a || i == b {
                continue;
            }
            // On its carriageway as the patch draws it (straight from the
            // vertex), or on its real one (its curving centre line, J6):
            // there on its surface. A side road leaving at a narrow angle
            // (a fork, a road along the main road) whose mouth lies within
            // SIDE_MARGIN_M of it starts on it too (where the main road
            // curves towards it, its strip covers the mouth).
            let narrow = [a, b]
                .iter()
                .any(|&m| dot(patch.arms[i].dir, patch.arms[m].dir) > ALONG_DEG.to_radians().cos());
            let margin = if narrow { SIDE_MARGIN_M } else { 0.0 };
            let on: [Option<f64>; 2] = std::array::from_fn(|k| {
                let q = xz(patch.arms[i].row[k]);
                if patch.main_excess(q) <= 0.0 {
                    patch.main_own(q)
                } else {
                    patch.main_road_height(g, q, MAIN_REACH_M, margin)
                }
            });
            // J6 (§4.8.8 Rule: a side road never above the main road): a
            // mouth with a corner on the main road and standing above it
            // starts on the main road – blended onto its surface as a side
            // road whose mouth lies at the main road's edge (J2): the
            // mouth row on the main road's surface, the strip turning from
            // it to its own profile past the mouth, the patch's surface
            // there the main road's (a path or a service road drawn along
            // the main road leaned up to its own height across the main
            // road's edge, its strip past the mouth was put under the main
            // road: between them the strip rose above it; lowering only
            // the mouth row left a step to the patch's surface).
            let above = (0..2).any(|k| on[k].is_some_and(|m| patch.arms[i].row[k][1] > m + 0.005));
            if !above {
                continue;
            }
            let row: [[f64; 3]; 2] = std::array::from_fn(|k| {
                let q = xz(patch.arms[i].row[k]);
                let own = shepard([&patch.arms[a], &patch.arms[b]].into_iter(), q);
                at_y(q, on[k].map_or(own, |m| m.min(own)))
            });
            let mid = lerp3(row[0], row[1], 0.5);
            let dh = (mid[1] - patch.arms[i].h).abs();
            let room = 0.5 * patch.arms[i].len - patch.arms[i].mouth_m;
            let mut l = (dh / BLEND_RATE).clamp(BLEND_MIN_M, BLEND_MAX_M).min(room);
            if l < 1.0 {
                l = 0.0;
            }
            let arm = &mut patch.arms[i];
            arm.edge_m = 0.0;
            arm.edge_p = [0.0, 0.0];
            arm.edge_h = 0.0;
            arm.corner = Vec::new();
            arm.cross = 0.0;
            arm.warp_m = 0.0;
            arm.row = row;
            arm.blend_m = l;
        }
    }
    Ok(patch)
}

/// The cross slopes of a blended patch's arms ([`Arm::cross`]): the least
/// squares plane through the arms' mouths – their heights, and their
/// grades along them weighted by the mouth's distance – drawn towards a
/// level plane where the arms leave little to fit (forks: their arms
/// nearly in a line), and each arm's slope across in it (at most
/// [`MAX_CROSS`]).
fn blend_plane(arms: &mut [Arm]) {
    if arms.len() < 3 {
        return;
    }
    // Rows of [1, x, z] · (h0, gx, gz) = value, with a weight.
    let c = arms.iter().fold([0.0, 0.0], |s, a| add(s, a.centre));
    let c = mul(c, 1.0 / arms.len() as f64);
    let mut m = [[0.0f64; 3]; 3];
    let mut b = [0.0f64; 3];
    let mut row = |r: [f64; 3], v: f64, w: f64| {
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += w * r[i] * r[j];
            }
            b[i] += w * r[i] * v;
        }
    };
    let mut ridge = 0.0;
    for a in arms.iter() {
        let q = sub(a.centre, c);
        row([1.0, q[0], q[1]], a.h, 1.0);
        let l = a.mouth_m.max(1.0);
        row([0.0, a.t[0], a.t[1]], a.grade, l * l);
        ridge += 0.25 * l * l;
    }
    m[1][1] += ridge;
    m[2][2] += ridge;
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    if d.abs() < 1e-12 {
        return;
    }
    let solve = |k: usize| {
        let mut mk = m;
        for (i, r) in mk.iter_mut().enumerate() {
            r[k] = b[i];
        }
        det(&mk) / d
    };
    let grad = [solve(1), solve(2)];
    for a in arms.iter_mut() {
        let cross = dot(grad, side_pos(a.t)).clamp(-MAX_CROSS, MAX_CROSS);
        a.lean(cross);
    }
}

/// The steepest cross slope a blended patch's arm takes inside it.
pub const MAX_CROSS: f64 = 0.12;

/// The steepest cross slope a mouth row leans by (J5, §4.8.8 Rule).
pub const MAX_WARP: f64 = 0.2;
/// A strip turns flat across over `sqrt(WARP_RATE × rise)` metres past the
/// mouth (`rise`: how far the row's corner lies off its centre's height),
/// at least [`WARP_MIN_M`], at most [`WARP_MAX_M`]: its edge bends by at
/// most 6 × rise / L² = 0.04 per metre (4 points).
pub const WARP_RATE: f64 = 150.0;
pub const WARP_MIN_M: f64 = 3.0;
pub const WARP_MAX_M: f64 = 20.0;

/// Where side arm `i`'s centre line leaves the through road's carriageway
/// ([`Patch::main_excess`]): metres from the vertex (within its mouth) and
/// the point.
fn edge_exit(g: &RoadGraph, patch: &Patch, i: usize) -> Option<(f64, P2)> {
    let arm = &patch.arms[i];
    let e = &g.edges[arm.edge];
    let cum = cumulative(&e.points);
    let at = |m: f64| frame_at(&e.points, &cum, arm.s_edge(m)).0;
    let step = 0.25;
    let mut lo = 0.0;
    let mut m = step;
    while m <= arm.mouth_m + 1e-9 {
        if patch.main_excess(at(m)) > 0.0 {
            // Between lo (on the carriageway) and m (off it).
            let mut hi = m;
            for _ in 0..30 {
                let mid = 0.5 * (lo + hi);
                if patch.main_excess(at(mid)) > 0.0 {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some((hi, at(hi)));
        }
        lo = m;
        m += step;
    }
    None
}

/// The arm's profile from the vertex to its mouth (metres from the vertex,
/// height): its samples, the edge point `edge_m` and the mouth.
fn corner_samples(g: &RoadGraph, arm: &Arm, edge_m: f64) -> Vec<(f64, f64)> {
    let e = &g.edges[arm.edge];
    let n = e.profile.len();
    let mut out: Vec<(f64, f64)> = (0..n)
        .map(|i| (i as f64 * PROFILE_STEP_M).min(arm.len))
        .map(|s| (if arm.at_from { s } else { arm.len - s }, s))
        .filter(|&(m, _)| m < arm.mouth_m - 1e-9)
        .map(|(m, s)| (m, height(e, s)))
        .collect();
    for m in [edge_m, arm.mouth_m] {
        out.push((m, height(e, arm.s_edge(m))));
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
    out
}

/// How far past its mouths a side road's strip is checked against the main
/// road's carriageway (m, §4.8.8 Q7).
pub const MAIN_REACH_M: f64 = 40.0;

/// A side road's strip rows within this distance of the main road's
/// carriageway lie under it too (their quads would reach onto it).
pub const SIDE_MARGIN_M: f64 = 1.0;

/// A side road leaving its main road at less than this angle (degrees)
/// runs along it: a fork, a path drawn beside it (J6).
pub const ALONG_DEG: f64 = 35.0;

/// A side road's strip lies this far under the main road's surface where
/// it crosses the main road's carriageway past its mouth (§4.8.8 Q7).
pub const SIDE_UNDER_M: f64 = 0.03;

/// A side arm's corner needs at least this much room between the main
/// road's edge and its mouth (m); with less it keeps J2's blend.
pub const CORNER_MIN_M: f64 = 0.5;

/// The sides' lift fades out this far in from the outline (m, J5).
pub const SIDE_LIFT_FADE_M: f64 = 1.5;
/// The sides between two fixed points follow a cubic curve where it stays
/// within this of the height field (m); else they are pinned to it at the
/// farthest point too (J5).
pub const SIDE_LIFT_MAX_M: f64 = 0.15;

/// A strip keeps no point of its own within this distance past a mouth
/// (m, J5): the mouth row and the next row square to the strip there would
/// cross where the road bends.
pub const MOUTH_CLEAR_M: f64 = 1.0;

/// An outline point as the package stores it (section `JNCT`): its offset
/// from the vertex in `f32`. The computed outline is rounded the same way,
/// so a stored patch and a computed one are the same to the bit.
fn quantised(c: P2, p: P2) -> P2 {
    [
        c[0] + f64::from((p[0] - c[0]) as f32),
        c[1] + f64::from((p[1] - c[1]) as f32),
    ]
}

/// [`build_patch`] with the reason when a vertex keeps its overlapping
/// strips (diagnostics, the analysis test).
fn try_patch(g: &RoadGraph, adj: &[usize], v: usize) -> Result<Patch, &'static str> {
    // J8: the mouths out for the corners, where that bends its sides no
    // more than before; where that patch cannot be made, as before.
    let plain = try_patch_with(g, adj, v, false);
    match (try_patch_with(g, adj, v, true), plain) {
        (Ok(grown), Ok(plain)) => Ok(less_bent(grown, plain)),
        (Ok(grown), Err(_)) => Ok(grown),
        (Err(_), plain) => plain,
    }
}

/// The worst bend of a patch's sides (radians) as the side lint measures
/// it: over pieces of 0.3 m, the arms' strip edges on at both ends of each
/// chain.
fn side_bend(p: &Patch) -> f64 {
    let mut worst = 0.0f64;
    for ch in &p.chains {
        let n = ch.points.len();
        if n < 2 {
            continue;
        }
        let (a, b) = (&p.arms[ch.from_arm], &p.arms[ch.to_arm]);
        let mut pts = vec![add(xz(ch.points[0]), a.t)];
        pts.extend(ch.points.iter().map(|q| xz(*q)));
        pts.push(add(xz(ch.points[n - 1]), b.t));
        worst = worst.max(sides::worst_bend(&pts));
    }
    worst
}

/// `new` unless its sides bend more than `old`'s (J8: a patch changed for
/// its corners never draws a sharper one).
fn less_bent(new: Patch, old: Patch) -> Patch {
    if side_bend(&new) <= side_bend(&old) + 1e-3 {
        new
    } else {
        old
    }
}

/// [`try_patch`] with the mouths moved out for the corners or not (J8).
fn try_patch_with(
    g: &RoadGraph,
    adj: &[usize],
    v: usize,
    grow: bool,
) -> Result<Patch, &'static str> {
    let (patch, outline) = patch_outline_with(g, adj, v, grow)?;
    finish(patch, outline)
}

/// The patch of [`try_patch`] before its outline is checked and meshed:
/// the arms and the outline points (rounded as stored).
#[cfg(test)]
fn patch_outline(g: &RoadGraph, adj: &[usize], v: usize) -> Result<(Patch, Outline), &'static str> {
    patch_outline_with(g, adj, v, true)
}

/// [`patch_outline`] with the mouths moved out for the corners or not.
fn patch_outline_with(
    g: &RoadGraph,
    adj: &[usize],
    v: usize,
    grow: bool,
) -> Result<(Patch, Outline), &'static str> {
    let c = g.vertices[v];
    let raws: Vec<Raw> = adj
        .iter()
        .map(|&ei| raw_arm(g, c, v, ei))
        .collect::<Result<_, _>>()?;
    let site = Site {
        grow,
        ..Site::single(g, v)
    };
    outline_checked(g, &site, raws, false)
}

/// The patch of a cluster (J3): vertices a few metres apart joined by the
/// short edges `inner`, with the other edges at them as its arms, round
/// their centroid.
fn try_cluster(
    g: &RoadGraph,
    adjacency: &[Vec<usize>],
    members: &[u32],
    inner: &[usize],
    crossings: Option<&Crossings>,
) -> Result<Patch, &'static str> {
    let mut c = [0.0, 0.0];
    for &m in members {
        c = add(c, g.vertices[m as usize]);
    }
    c = mul(c, 1.0 / members.len() as f64);
    let mut raws = Vec::new();
    for &m in members {
        let v = m as usize;
        for &ei in &adjacency[v] {
            if !inner.contains(&ei) {
                raws.push(raw_arm(g, g.vertices[v], v, ei)?);
            }
        }
    }
    if raws.len() < 2 || raws.len() > MAX_ARMS {
        return Err("a cluster with too many arms");
    }
    let site = Site {
        centre: c,
        members: members.to_vec(),
        inner: inner.to_vec(),
        run_on: Vec::new(),
        grow: false,
        free: false,
    };
    // J7's corners between arms at different members first (their edge
    // lines meeting behind them, straight on through the cluster); where
    // that costs the patch, J6's.
    // J7's corners between arms at different members (their edge lines
    // meeting behind them, straight on through the cluster). Where that
    // cannot be made, each member keeps its own rule: tried, J6's cluster
    // outline instead left 57 more `side.corner` in Liechtenstein.
    let star = outline_checked(g, &site, raws.clone(), true)
        .and_then(|(patch, outline)| finish(patch, outline));
    match star {
        Err("the outline is not star-shaped") => {
            // J9: where no star-shaped outline can be made, any simple one
            // (as a junction's own patch since J6) – the short edges
            // between the members shared, not squeezed by both.
            // Only where its sides bend no more than the lint allows, or
            // than the members' own patches do.
            let site = Site { free: true, ..site };
            let (patch, outline) = outline_at(g, &site, raws, false, true)?;
            let patch = finish(patch, outline)?;
            let owns: Vec<Patch> = members
                .iter()
                .filter_map(|&m| {
                    let m = m as usize;
                    try_patch(g, &adjacency[m], m).ok()
                })
                .collect();
            let own = owns
                .iter()
                .map(side_bend)
                .fold(FREE_CLUSTER_BEND_DEG.to_radians(), f64::max);
            // Nor where it swallows a painted crossing the members' own
            // patches leave on its strip.
            let kept = count_swallowed(crossings, g, &patch)
                <= owns
                    .iter()
                    .map(|p| count_swallowed(crossings, g, p))
                    .sum::<usize>();
            // (On arms at most 6 % steep: on steeper ones the plane over
            // both members kinks the roads, +10 % road kinks in
            // Liechtenstein.)
            let flat = patch.arms.iter().all(|a| a.grade.abs() <= GROW_MAX_GRADE);
            if flat && kept && side_bend(&patch) <= own + 1e-3 {
                Ok(patch)
            } else {
                Err("the outline is not star-shaped")
            }
        }
        r => r,
    }
}

/// How many of the painted `crossings` (per edge, J9) patch `p` swallows:
/// those on its arms' edges whose painted length starts within the arm's
/// mouth, and every one on an edge inside it.
fn count_swallowed(crossings: Option<&Crossings>, g: &RoadGraph, p: &Patch) -> usize {
    let Some(cs) = crossings else {
        return 0;
    };
    let mut n = 0;
    for a in &p.arms {
        let len = g.edges[a.edge].length_m();
        n += cs.get(a.edge).map_or(0, |list| {
            list.iter()
                .filter(|&&(s, half)| {
                    let d = if a.at_from { s } else { len - s };
                    d - half < a.mouth_m
                })
                .count()
        });
    }
    for &ei in &p.inner {
        n += cs.get(ei).map_or(0, Vec::len);
    }
    n
}

/// An arm's mouth may lie this far before what its edge's far end takes
/// (m; J9, [`JunctionSet::far_room`]).
pub const ROOM_MARGIN_M: f64 = 1.0;

/// A cluster's simple outline (J9) may bend its sides by this much (the
/// lint's `side.corner`, degrees), or as much as its members' own patches.
const FREE_CLUSTER_BEND_DEG: f64 = 25.0;

/// [`outline_at`] with J5's sides, where J4's straight-line outline is
/// star-shaped too: a junction J1 – J4 left without a surface keeps its
/// overlapping strips (J3 tried such surfaces: more kinks and seams).
fn outline_checked(
    g: &RoadGraph,
    site: &Site,
    raws: Vec<Raw>,
    j7: bool,
) -> Result<(Patch, Outline), &'static str> {
    let (_, legacy) = outline_at(g, site, raws.clone(), true, j7)?;
    let pts: Vec<P2> = legacy.iter().map(|o| o.0).collect();
    if !sides::star_ok(site.centre, &pts) {
        return Err("the outline is not star-shaped");
    }
    outline_at(g, site, raws, false, j7)
}

/// The arms' mouths and the outline of the patch at `site` (rounded as
/// stored); `legacy`: the chains between the mouths as J1 – J4 drew them.
fn outline_at(
    g: &RoadGraph,
    site: &Site,
    mut raws: Vec<Raw>,
    legacy: bool,
    j7: bool,
) -> Result<(Patch, Outline), &'static str> {
    let c = site.centre;
    let cluster = site.cluster();
    // J6: outlines need not be star-shaped, only simple (their mesh then
    // comes from `plane_mesh`); clusters keep J5's star-shaped choice
    // (their chains round the centre between two carriageways leave
    // seams with the strips beside a median when drawn otherwise).
    let general = !legacy && (!cluster || site.free);
    // A cluster's arms by the polar angle of a point along them round its
    // centroid; a vertex's by their direction.
    let polar = |r: &Raw| {
        // (An arm run on past a way split, J8: by the point along it too.)
        if cluster || !r.lead.is_empty() {
            angle(c, r.refp)
        } else {
            r.dir[1].atan2(r.dir[0])
        }
    };
    raws.sort_by(|a, b| polar(a).total_cmp(&polar(b)).then(a.edge.cmp(&b.edge)));
    let n = raws.len();
    let tau = std::f64::consts::TAU;
    let wrap = |mut d: f64| {
        while d <= 0.0 {
            d += tau;
        }
        d
    };
    // The angle from arm i to the next: round the centre, and between the
    // arms' directions (the same at one vertex).
    let gap = |i: usize| wrap(polar(&raws[(i + 1) % n]) - polar(&raws[i]));
    let turn = |i: usize| {
        let (a, b) = (raws[i].dir, raws[(i + 1) % n].dir);
        wrap(b[1].atan2(b[0]) - a[1].atan2(a[0]))
    };
    if (0..n).any(|i| gap(i) < MIN_CORNER_DEG.to_radians()) {
        return Err("a fork: arms closer than 4 degrees");
    }
    let fork_arm =
        |i: usize| gap(i) < FORK_DEG.to_radians() || gap((i + n - 1) % n) < FORK_DEG.to_radians();
    let caps: Vec<f64> = raws
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let max = if fork_arm(i) {
                FORK_MAX_MOUTH_M
            } else {
                MAX_MOUTH_M
            };
            max.min(r.room.unwrap_or(MOUTH_SHARE * r.len))
        })
        .collect();
    // Concave corners.
    let mut corners: Vec<Option<CornerArc>> = Vec::with_capacity(n);
    let mut need = vec![1.0f64; n];
    for (i, r) in raws.iter().enumerate() {
        // The widest other road at its vertex (in a cluster: at its own
        // member, the edges inside included).
        let mut other = raws
            .iter()
            .enumerate()
            .filter(|(k, o)| *k != i && (!cluster || o.origin == r.origin))
            .map(|(_, o)| o.half)
            .fold(0.0, f64::max);
        for &ei in &site.inner {
            let e = &g.edges[ei];
            if [e.from, e.to]
                .iter()
                .any(|w| g.vertices[w.0 as usize] == r.origin)
            {
                other = other.max(e.width_m / 2.0);
            }
        }
        need[i] = need[i].max(other).max(r.half.min(1.0));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let phi = if cluster { turn(i) } else { gap(i) };
        if phi >= std::f64::consts::PI - 0.05 {
            corners.push(None);
            continue;
        }
        let (ri, rj) = (&raws[i], &raws[j]);
        let li = add(ri.origin, mul(side_pos(ri.dir), ri.half));
        let lj = add(rj.origin, mul(side_neg(rj.dir), rj.half));
        // In a cluster the edge lines of two arms at different members may
        // meet behind one of them (J7): on the inner edge it goes on from
        // (a street through the cluster to the main road), within the
        // cluster's reach.
        let behind = if j7 && cluster && ri.origin != rj.origin {
            -CLUSTER_EDGE_M
        } else {
            0.0
        };
        let hit = intersect(li, ri.dir, lj, rj.dir).filter(|&(_, si, sj)| {
            si <= caps[i] - 0.2 && sj <= caps[j] - 0.2 && (!cluster || (si > behind && sj > behind))
        });
        let Some((x, sx_i, sx_j)) = hit else {
            // Nearly straight (a width change): the edge lines meet far
            // out, the outline runs straight across; an acute corner that
            // far out is a fork the patch cannot hold. In a cluster the
            // outline runs straight from mouth to mouth (two carriageways
            // side by side).
            if phi > 2.0 || cluster {
                corners.push(None);
                continue;
            }
            return Err("an acute corner beyond the mouth limits");
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
    // J8: the mouths far enough out for each corner's radius along the
    // roads' real edge lines (the corner arcs above take the straight
    // lines from the vertex: where an arm curves or runs on past a way
    // split its corner was cut short at the mouth), within the caps.
    if !legacy && !cluster && site.grow {
        for _ in 0..MOUTH_ROUNDS {
            let arms: Vec<Arm> = raws
                .iter()
                .enumerate()
                .map(|(i, r)| make_arm(g, r, need[i].min(caps[i]), Some(0.0)))
                .collect();
            let mut grew = false;
            for i in 0..n {
                let j = (i + 1) % n;
                let radius = corner_radius(arms[i].class.max(arms[j].class));
                let d = sides::corner_deficit(g, &arms[i], &arms[j], radius, None, [false; 2]);
                for (k, dk) in [(i, d[0]), (j, d[1])] {
                    let m = need[k].min(caps[k]);
                    // Not on a steep arm: its mouth farther out leaves its
                    // blend less room (road kinks, Tenerife).
                    if dk > 0.05 && m < caps[k] - 0.05 && arms[k].grade.abs() <= GROW_MAX_GRADE {
                        need[k] = (m + dk).min(caps[k]);
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
    }
    // Mouths.
    let arms: Vec<Arm> = raws
        .iter()
        .enumerate()
        .map(|(i, r)| make_arm(g, r, need[i].min(caps[i]), None))
        .collect();
    let patch = assemble(g, site, arms)?;
    // The outline: per arm its mouth (P to Q), then the chain to the next –
    // per corner the chains it may take, the first preferred (J6: J5's
    // sides wherever the outline stays a simple polygon, star-shaped or
    // not; else J4's line, smoothed or not).
    // Whether an arm's edge goes on straight (within 20°) through its
    // cluster member along an edge inside the cluster (J7).
    let straight_on = |arm: &Arm| {
        site.inner.iter().any(|&ei| {
            let e = &g.edges[ei];
            let at_origin = |w: VertexId| g.vertices[w.0 as usize] == arm.origin;
            let d = if at_origin(e.from) {
                unit(sub(e.points[1], e.points[0]))
            } else if at_origin(e.to) {
                let n = e.points.len();
                unit(sub(e.points[n - 2], e.points[n - 1]))
            } else {
                return false;
            };
            dot(d, mul(arm.dir, -1.0)) > 20f64.to_radians().cos()
        })
    };
    let mut mouths: Vec<Vec<P2>> = Vec::with_capacity(n);
    let mut choices: Vec<Vec<Vec<P2>>> = Vec::with_capacity(n);
    for (i, corner) in corners.iter().enumerate() {
        let j = (i + 1) % n;
        let arm = &patch.arms[i];
        let (p, q) = (arm.p(), arm.q());
        let pieces = (norm(sub(xz(q), xz(p))) / MOUTH_STEP_M).ceil().max(1.0) as usize;
        mouths.push(
            (0..=pieces)
                .map(|k| xz(lerp3(p, q, k as f64 / pieces as f64)))
                .collect(),
        );
        let mut options: Vec<Vec<P2>> = Vec::new();
        // The chain from Q_i to P_j: along the edge lines and round the
        // corner (§4.8.8 Rule, J5); the straight-line chain of J1 – J4
        // where that would not keep the outline a simple polygon (J5: not
        // star-shaped).
        let next = &patch.arms[j];
        let radius = corner_radius(arm.class.max(next.class));
        if !legacy
            && let Some(chain) = sides::corner_chain(
                g,
                arm,
                next,
                radius,
                (cluster && arm.origin != next.origin).then_some(c),
                [j7 && straight_on(arm), j7 && straight_on(next)],
            )
            && (general || sides::polar_ok(c, &chain))
        {
            let last = chain.len() - 1;
            options.push(chain[1..last].to_vec());
        }
        let mut pts2: Vec<P2> = vec![xz(q)];
        match corner {
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
            None if cluster && arm.origin != next.origin => {
                // Two arms of different vertices (two carriageways side by
                // side, the entries of a small roundabout): round the
                // centre from mouth to mouth, outside the edges inside.
                let (a, b) = (xz(q), xz(next.p()));
                let (r0, r1) = (norm(sub(a, c)), norm(sub(b, c)));
                let a0 = angle(c, a);
                let mut a1 = angle(c, b);
                while a1 <= a0 {
                    a1 += tau;
                }
                // Only forward round the centre (else straight across).
                let steps = if a1 - a0 < std::f64::consts::PI {
                    ((a1 - a0) / ARC_STEP_DEG.to_radians()).ceil().max(1.0) as usize
                } else {
                    1
                };
                for k in 1..steps {
                    let f = k as f64 / steps as f64;
                    let (ang, r) = (a0 + (a1 - a0) * f, r0 + (r1 - r0) * f);
                    pts2.push(add(c, [r * ang.cos(), r * ang.sin()]));
                }
            }
            None => {
                // Straight or convex: through the edge lines' crossing where
                // it lies between the mouths.
                let li = add(arm.origin, mul(side_pos(arm.dir), arm.half));
                let lj = add(next.origin, mul(side_neg(next.dir), next.half));
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
        // Smoothed where that keeps it star-shaped (J5) or simple (J6):
        // leaving and reaching the mouths along their arms, its bends
        // rounded.
        if !legacy
            && let Some(smooth) = sides::smoothed(&dense, arm.t, next.t)
            && (general || sides::polar_ok(c, &smooth))
        {
            let last = smooth.len() - 1;
            options.push(smooth[1..last].to_vec());
        }
        let last = dense.len() - 1;
        options.push(dense[1..last].to_vec());
        // J6: the chains ordered by their worst bend (as the side lint
        // measures it, the strips' edges on at both ends), J5's sides first
        // where they bend no more than the others.
        if general && options.len() > 1 {
            let (q2, p2) = (xz(q), xz(next.p()));
            let score = |o: &Vec<P2>| {
                let mut pts = vec![add(q2, arm.t), q2];
                pts.extend(o.iter().copied());
                pts.push(p2);
                pts.push(add(p2, next.t));
                sides::worst_bend(&pts)
            };
            let mut scored: Vec<(f64, usize, Vec<P2>)> = options
                .into_iter()
                .enumerate()
                .map(|(k, o)| (score(&o), k, o))
                .collect();
            // Within a degree the earlier (J5's sides, then smoothed) wins.
            scored.sort_by(|a, b| {
                let (x, y) = (a.0.to_degrees().round(), b.0.to_degrees().round());
                x.total_cmp(&y).then(a.1.cmp(&b.1))
            });
            options = scored.into_iter().map(|x| x.2).collect();
        }
        choices.push(options);
    }
    // Each corner's first choice; where the outline crosses itself, the
    // corners whose chains cross take their next (J6).
    let mut pick = vec![0usize; n];
    let assemble_outline = |pick: &[usize]| -> (Vec<(P2, Option<usize>)>, Vec<usize>) {
        let mut out: Vec<(P2, Option<usize>)> = Vec::new();
        let mut owner: Vec<usize> = Vec::new();
        for i in 0..n {
            for &p2 in &mouths[i] {
                out.push((p2, Some(i)));
                owner.push(i);
            }
            for &p2 in &choices[i][pick[i]] {
                out.push((p2, None));
                owner.push(i);
            }
        }
        (out, owner)
    };
    let (mut outline, mut owner) = assemble_outline(&pick);
    if general {
        for _ in 0..=2 * n {
            let pts: Vec<P2> = outline.iter().map(|o| o.0).collect();
            let bad = plane_mesh::crossing_pieces(&pts);
            if bad.is_empty() {
                break;
            }
            let mut changed = false;
            let mut corners_bad: Vec<usize> = bad.iter().map(|&k| owner[k]).collect();
            corners_bad.sort_unstable();
            corners_bad.dedup();
            for i in corners_bad {
                if pick[i] + 1 < choices[i].len() {
                    pick[i] += 1;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
            (outline, owner) = assemble_outline(&pick);
        }
    }
    let outline = outline
        .into_iter()
        .map(|(p, m)| (quantised(c, p), m))
        .collect();
    Ok((patch, outline))
}

/// Outline points and the arm whose mouth each lies on.
type Outline = Vec<(P2, Option<usize>)>;

/// The rest of a patch from its (rounded) outline: the heights of the
/// outline points (on a mouth the arm's row, elsewhere the field), the
/// chains between the mouths, the star check and the mesh.
fn finish(mut patch: Patch, outline2: Vec<(P2, Option<usize>)>) -> Result<Patch, &'static str> {
    let c = xz(patch.centre);
    let n = outline2.len();
    if n < 3 || patch.arms.is_empty() {
        return Err("the outline is not star-shaped");
    }
    let mut outline: Vec<OutlinePoint> = Vec::with_capacity(n);
    for &(p2, mouth) in &outline2 {
        let y = match mouth {
            Some(i) => {
                let a = patch.arms.get(i).ok_or("the outline is not star-shaped")?;
                let (r0, r1) = (a.row[0], a.row[1]);
                let d = sub(xz(r1), xz(r0));
                let l2 = dot(d, d);
                let t = if l2 > 0.0 {
                    (dot(sub(p2, xz(r0)), d) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                r0[1] + (r1[1] - r0[1]) * t
            }
            None => patch.field(p2),
        };
        outline.push(OutlinePoint {
            p: at_y(p2, y),
            mouth,
        });
    }
    // The chains: from the last mouth point of each arm round to the first
    // of the next.
    let na = patch.arms.len();
    let mut chains = Vec::with_capacity(na);
    for i in 0..na {
        let j = (i + 1) % na;
        let Some(last) = (0..n).rev().find(|&k| outline[k].mouth == Some(i)) else {
            return Err("the outline is not star-shaped");
        };
        let mut points = vec![outline[last].p];
        let mut k = (last + 1) % n;
        let mut guard = 0;
        while outline[k].mouth.is_none() {
            points.push(outline[k].p);
            k = (k + 1) % n;
            guard += 1;
            if guard > n {
                return Err("the outline is not star-shaped");
            }
        }
        if outline[k].mouth != Some(j) {
            return Err("the outline is not star-shaped");
        }
        points.push(outline[k].p);
        chains.push(Chain {
            from_arm: i,
            to_arm: j,
            points,
        });
    }
    // Star-shaped from the centre: polar angles strictly growing, once
    // round; else (J6) a simple polygon, meshed by `plane_mesh`.
    let star = match star_angles(c, &outline) {
        Ok(angles) => Some(angles),
        Err(e) => {
            let pts: Vec<P2> = outline.iter().map(|o| xz(o.p)).collect();
            if !plane_mesh::simple_ccw(&pts) {
                return Err(e);
            }
            None
        }
    };
    patch.outline = outline;
    patch.angles = star.clone().unwrap_or_default();
    side_lift(&mut patch, &chains);
    // The chains with the lifted heights.
    patch.chains = chains
        .into_iter()
        .map(|mut ch| {
            for q in &mut ch.points {
                q[1] = patch.outline_height(*q);
            }
            ch
        })
        .collect();
    if star.is_some() {
        mesh(&mut patch);
    } else {
        plane(&mut patch)?;
    }
    Ok(patch)
}

/// The outline points' polar angles round `c`, unwrapped, if the outline
/// is star-shaped from it.
fn star_angles(c: P2, outline: &[OutlinePoint]) -> Result<Vec<f64>, &'static str> {
    let tau = std::f64::consts::TAU;
    let mut angles = Vec::with_capacity(outline.len());
    let mut prev = angle(c, xz(outline[0].p));
    angles.push(prev);
    for o in &outline[1..] {
        if norm(sub(xz(o.p), c)) < 0.3 {
            return Err("the outline passes the vertex");
        }
        let mut a = angle(c, xz(o.p));
        while a <= prev - 1e-12 {
            a += tau;
        }
        if a - prev < 1e-6 || a - prev > std::f64::consts::PI {
            return Err("the outline is not star-shaped");
        }
        angles.push(a);
        prev = a;
    }
    if prev - angles[0] >= tau - 1e-6 {
        return Err("the outline is not star-shaped");
    }
    let first = angles[0] + tau;
    if first - prev < 1e-6 || first - prev > std::f64::consts::PI {
        return Err("the outline is not star-shaped");
    }
    Ok(angles)
}

/// The mesh of an outline that is not star-shaped (J6, `plane_mesh`): a
/// constrained Delaunay triangulation of the outline and points inside it
/// at the surface's heights, cut at the through road's carriageway edge
/// as the star mesh.
fn plane(patch: &mut Patch) -> Result<(), &'static str> {
    let pts: Vec<P2> = patch.outline.iter().map(|o| xz(o.p)).collect();
    let mut inner = plane_mesh::inner_points(&pts);
    // Points along the through road's carriageway edges (every 0.5 m),
    // so that the triangles follow them and the cut there (below) leaves
    // no triangle of a side road's corner reaching onto the main road (the
    // star mesh's rings do so by their spacing).
    if let Some([a, b]) = patch.through {
        for k in [a, b] {
            let arm = &patch.arms[k];
            let len = arm.mouth_m + 1.0;
            let steps = (len / 0.5).ceil() as usize;
            for side in [-1.0, 1.0] {
                let off = mul(side_pos(arm.dir), side * arm.half);
                for s in 0..=steps {
                    let q = add(
                        add(arm.origin, mul(arm.dir, len * s as f64 / steps as f64)),
                        off,
                    );
                    if plane_mesh::inside(&pts, q)
                        && plane_mesh::nearest_piece(&pts, q).0 > 0.1
                        && inner.iter().all(|w| norm(sub(*w, q)) > 0.2)
                    {
                        inner.push(q);
                    }
                }
            }
        }
    }
    let mut tris =
        plane_mesh::triangulate(&pts, &inner).ok_or("the outline cannot be triangulated")?;
    let mut verts: Vec<[f64; 3]> = patch.outline.iter().map(|o| o.p).collect();
    for q in &inner {
        verts.push(at_y(*q, patch.surface(*q)));
    }
    let c = xz(patch.centre);
    patch.centre[1] = patch.field(c);
    let (mut lo, mut hi) = (pts[0], pts[0]);
    for q in &pts {
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    patch.min = lo;
    patch.max = hi;
    let mut sector = vec![0usize; tris.len()];
    if patch.through.is_some() {
        for _ in 0..REFINE_ROUNDS {
            (tris, sector) = refine_at_main_edge(patch, &mut verts, tris, sector);
        }
        (tris, _) = cut_at_main_edge(patch, &mut verts, tris, sector);
    }
    for t in &mut tris {
        let [a, b, cc] = t.map(|i| verts[i as usize]);
        let up = (b[2] - a[2]) * (cc[0] - a[0]) - (b[0] - a[0]) * (cc[2] - a[2]);
        if up < 0.0 {
            t.swap(1, 2);
        }
    }
    patch.cells = Some(plane_mesh::CellIndex::of(&verts, &tris, lo, hi));
    patch.vertices = verts;
    patch.triangles = tris;
    patch.sectors = Vec::new();
    Ok(())
}

impl Patch {
    /// The (lifted) height of the outline point at `q` (x, z of an outline
    /// point).
    fn outline_height(&self, q: [f64; 3]) -> f64 {
        self.outline
            .iter()
            .find(|o| o.p[0] == q[0] && o.p[2] == q[2])
            .map_or(q[1], |o| o.p[1])
    }
}

/// The sides' lift (J5, §4.8.8 Rule: the sides as smooth as the road):
/// along each chain between two mouths the outline's heights follow a
/// cubic curve (C1 with the field's slope along the chain) between fixed
/// points – the mouth corners, the points on the through road's
/// carriageway (the main road keeps its surface) and, where the curve
/// would leave the field by more than [`SIDE_LIFT_MAX_M`], the farthest
/// point (again, until it does not): the field's S-shaped blends between
/// two roads' surfaces become even slopes. The outline points move by the
/// difference; [`Patch::surface`] follows them inward.
fn side_lift(patch: &mut Patch, chains: &[Chain]) {
    let n = patch.outline.len();
    let mut lift = vec![0.0f64; n];
    let index: std::collections::HashMap<(u64, u64), usize> = patch
        .outline
        .iter()
        .enumerate()
        .map(|(k, o)| ((o.p[0].to_bits(), o.p[2].to_bits()), k))
        .collect();
    for ch in chains {
        let m = ch.points.len();
        if m < 3 {
            continue;
        }
        let pts: Vec<P2> = ch.points.iter().map(|q| xz(*q)).collect();
        let f: Vec<f64> = ch.points.iter().map(|q| q[1]).collect();
        let mut u = vec![0.0f64; m];
        for i in 1..m {
            u[i] = u[i - 1] + norm(sub(pts[i], pts[i - 1]));
        }
        // The field's slope along the chain at point i.
        let slope = |i: usize| {
            let d = if i == 0 {
                sub(pts[1], pts[0])
            } else if i == m - 1 {
                sub(pts[m - 1], pts[m - 2])
            } else {
                sub(pts[i + 1], pts[i - 1])
            };
            let d = unit(d);
            let e = 0.05;
            (patch.field(add(pts[i], mul(d, e))) - patch.field(sub(pts[i], mul(d, e)))) / (2.0 * e)
        };
        let mut fixed = vec![false; m];
        fixed[0] = true;
        fixed[m - 1] = true;
        if patch.through.is_some() {
            for (x, q) in fixed.iter_mut().zip(&pts) {
                if patch.main_excess(*q) <= 0.05 {
                    *x = true;
                }
            }
        }
        let mut h = f.clone();
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut a = 0;
        for (i, &x) in fixed.iter().enumerate().skip(1) {
            if x {
                if i > a + 1 {
                    runs.push((a, i));
                }
                a = i;
            }
        }
        while let Some((a, b)) = runs.pop() {
            let l = u[b] - u[a];
            if l < 1e-6 {
                continue;
            }
            let (sa, sb) = (slope(a), slope(b));
            let mut worst = (0.0f64, a);
            for i in a + 1..b {
                let t = (u[i] - u[a]) / l;
                h[i] = hermite(t, f[a], sa * l, f[b], sb * l);
                let dev = (h[i] - f[i]).abs();
                if dev > worst.0 {
                    worst = (dev, i);
                }
            }
            if worst.0 > SIDE_LIFT_MAX_M {
                let k = worst.1;
                h[k] = f[k];
                runs.push((a, k));
                runs.push((k, b));
            }
        }
        for i in 1..m - 1 {
            if let Some(&k) = index.get(&(pts[i][0].to_bits(), pts[i][1].to_bits())) {
                lift[k] = h[i] - f[i];
            }
        }
    }
    for (o, l) in patch.outline.iter_mut().zip(&lift) {
        o.p[1] += l;
    }
    patch.lift = lift;
}

/// A patch from its stored outline (section `JNCT`): the arms (edge, end,
/// mouth, along) and the outline points; the same as [`try_patch`] gives.
fn stored_patch(g: &RoadGraph, v: usize, rec: &section::StoredPatch) -> Option<Patch> {
    g.vertices.get(v)?;
    let site = if rec.members.len() > 1 {
        // The centre: the centroid of the junctions (a cluster's, J3), not
        // of the way splits its arms run on past (J8).
        let mut c = [0.0, 0.0];
        let mut k = 0usize;
        for &m in &rec.members {
            let p = *g.vertices.get(m as usize)?;
            if !rec.run_on.contains(&m) {
                c = add(c, p);
                k += 1;
            }
        }
        if rec.inner.iter().any(|&e| e as usize >= g.edges.len())
            || rec.members[0] as usize != v
            || k == 0
            || rec.run_on.iter().any(|w| !rec.members.contains(w))
        {
            return None;
        }
        Site {
            centre: mul(c, 1.0 / k as f64),
            members: rec.members.clone(),
            inner: rec.inner.iter().map(|&e| e as usize).collect(),
            run_on: rec.run_on.clone(),
            grow: true,
            free: false,
        }
    } else {
        Site::single(g, v)
    };
    let c = site.centre;
    let mut arms = Vec::with_capacity(rec.arms.len());
    for a in &rec.arms {
        let e = g.edges.get(a.edge as usize)?;
        let at = (if a.at_from { e.from } else { e.to }).0;
        if !site.members.contains(&at) || e.from == e.to {
            return None;
        }
        let r = if site.run_on.contains(&at) {
            // Run on past the split `at`: the edge inside from the vertex.
            let at = at as usize;
            let lead = site.inner.iter().copied().find(|&ei| {
                let x = &g.edges[ei];
                let (f, t) = (x.from.0 as usize, x.to.0 as usize);
                (f == v && t == at) || (f == at && t == v)
            })?;
            raw_run_on(g, v, lead, at, a.edge as usize).ok()?
        } else {
            let at = at as usize;
            raw_arm(g, g.vertices[at], at, a.edge as usize).ok()?
        };
        arms.push(make_arm(g, &r, a.mouth_m, Some(a.along)));
    }
    let patch = assemble(g, &site, arms).ok()?;
    let outline = rec
        .outline
        .iter()
        .map(|o| {
            (
                [c[0] + f64::from(o.d[0]), c[1] + f64::from(o.d[1])],
                o.mouth.map(usize::from),
            )
        })
        .collect();
    finish(patch, outline).ok()
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
    // Rings about RING_M apart, and two more close to the outline (J3):
    // the corners between a main road's edge and a side road's mouth
    // curve there, the mesh meets the strips with their slope.
    let nr = ((reach / RING_M).ceil() as usize).clamp(2, 10);
    let mut rings: Vec<f64> = (0..nr).map(|j| 1.0 - j as f64 / nr as f64).collect();
    for d in [OUTER_RING_M, 2.5 * OUTER_RING_M] {
        let r = 1.0 - d / reach.max(1e-6);
        if r > rings.get(1).copied().unwrap_or(0.0) + 0.02 {
            rings.push(r);
        }
    }
    rings.sort_by(|a, b| b.total_cmp(a));
    let nr = rings.len();
    let mut verts = Vec::with_capacity(nr * n + 1);
    for (j, &r) in rings.iter().enumerate() {
        for o in &patch.outline {
            if j == 0 {
                verts.push(o.p);
            } else {
                let p2 = add(c, mul(sub(xz(o.p), c), r));
                let k = verts.len() % n;
                let lift = patch.lift.get(k).map_or(0.0, |&l| {
                    patch.lift_on(l, (1.0 - r) * norm(sub(xz(o.p), c)), p2)
                });
                verts.push(at_y(p2, patch.field(p2) + lift));
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
    // Per sector (between outline points k and k + 1) its triangles.
    let mut tris = Vec::with_capacity(2 * n * nr);
    let mut sector = Vec::with_capacity(2 * n * nr);
    for k in 0..n {
        for j in 0..nr {
            let k1 = (k + 1) % n;
            let (o0, o1, i0, i1) = (idx(j, k), idx(j, k1), idx(j + 1, k), idx(j + 1, k1));
            tris.push([o0, o1, i1]);
            sector.push(k);
            if j + 1 < nr {
                tris.push([o0, i1, i0]);
                sector.push(k);
            }
        }
    }
    // A through road with side roads' corners (§4.8.8 Q6 (d)): the
    // triangles crossing its carriageway's edge are cut there (§4.8.8 Q7),
    // so the corners' slope never reaches onto the main road's surface.
    if patch.through.is_some() {
        for _ in 0..REFINE_ROUNDS {
            (tris, sector) = refine_at_main_edge(patch, &mut verts, tris, sector);
        }
        (tris, sector) = cut_at_main_edge(patch, &mut verts, tris, sector);
    }
    // Facing up (as the road strips: cross(b − a, c − a).y > 0).
    for t in &mut tris {
        let [a, b, cc] = t.map(|i| verts[i as usize]);
        let up = (b[2] - a[2]) * (cc[0] - a[0]) - (b[0] - a[0]) * (cc[2] - a[2]);
        if up < 0.0 {
            t.swap(1, 2);
        }
    }
    let mut sectors = vec![Vec::new(); n];
    for (ti, &k) in sector.iter().enumerate() {
        sectors[k].push(ti as u32);
    }
    patch.vertices = verts;
    patch.triangles = tris;
    patch.sectors = sectors;
}

/// The triangles near the through road's carriageway edge are halved this
/// many times before they are cut there (§4.8.8 Q7): the edge of a
/// cluster's carriageways (the union of their strips) bends inside a
/// triangle of the star mesh.
const REFINE_ROUNDS: usize = 2;

/// Splits every triangle side whose corners or middle lie on different
/// sides of the through road's carriageway edge at its middle (a shared
/// vertex at the field's height), and each triangle by its split sides
/// (one: two triangles, two: three, three: four), so the mesh stays
/// closed. Returns the triangles with their sectors.
fn refine_at_main_edge(
    patch: &Patch,
    verts: &mut Vec<[f64; 3]>,
    tris: Vec<[u32; 3]>,
    sector: Vec<usize>,
) -> (Vec<[u32; 3]>, Vec<usize>) {
    use std::collections::HashMap;
    let on = |q: [f64; 3]| on_main(patch, xz(q));
    let mut mids: HashMap<(u32, u32), Option<u32>> = HashMap::new();
    for t in &tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let key = (a.min(b), a.max(b));
            if mids.contains_key(&key) {
                continue;
            }
            let (pa, pb) = (verts[a as usize], verts[b as usize]);
            let m = xz(lerp3(pa, pb, 0.5));
            let split = on(pa) != on(pb) || on(pa) != on_main(patch, m);
            let id = split.then(|| {
                verts.push(at_y(m, patch.surface(m)));
                (verts.len() - 1) as u32
            });
            mids.insert(key, id);
        }
    }
    let mid = |a: u32, b: u32| mids[&(a.min(b), a.max(b))];
    let mut out = Vec::with_capacity(tris.len() * 2);
    let mut out_sector = Vec::with_capacity(tris.len() * 2);
    for (t, sec) in tris.into_iter().zip(sector) {
        let m = [mid(t[0], t[1]), mid(t[1], t[2]), mid(t[2], t[0])];
        let pieces: Vec<[u32; 3]> = match m {
            [None, None, None] => vec![t],
            [Some(a), Some(b), Some(c)] => {
                vec![[t[0], a, c], [a, t[1], b], [c, b, t[2]], [a, b, c]]
            }
            _ => {
                // Rotate so the first split side is side 0 (t0–t1).
                let r = (0..3)
                    .find(|&k| m[k].is_some() && m[(k + 2) % 3].is_none())
                    .unwrap_or(0);
                let (v0, v1, v2) = (t[r], t[(r + 1) % 3], t[(r + 2) % 3]);
                let (m01, m12) = (m[r].expect("split"), m[(r + 1) % 3]);
                match m12 {
                    None => vec![[v0, m01, v2], [m01, v1, v2]],
                    Some(m12) => vec![[v0, m01, v2], [m01, v1, m12], [m01, m12, v2]],
                }
            }
        };
        for q in pieces {
            out.push(q);
            out_sector.push(sec);
        }
    }
    (out, out_sector)
}

/// On the through road's carriageway (within rounding).
fn on_main(patch: &Patch, p: P2) -> bool {
    patch.main_excess(p) <= 1e-6
}

/// Cuts the triangles that have corners on and off the through road's
/// carriageway at its edge ([`Patch::main_excess`]): the edge's crossing
/// of each such triangle side becomes a vertex (shared by both triangles
/// of the side, at the field's height there: the main road's surface),
/// the triangle a triangle and a quad. Returns the triangles with their
/// sectors.
fn cut_at_main_edge(
    patch: &Patch,
    verts: &mut Vec<[f64; 3]>,
    tris: Vec<[u32; 3]>,
    sector: Vec<usize>,
) -> (Vec<[u32; 3]>, Vec<usize>) {
    let on: Vec<bool> = verts.iter().map(|q| on_main(patch, xz(*q))).collect();
    let mut cuts: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    let mut cut = |verts: &mut Vec<[f64; 3]>, a: u32, b: u32| -> u32 {
        let key = (a.min(b), a.max(b));
        if let Some(&i) = cuts.get(&key) {
            return i;
        }
        // From the corner on the carriageway to the one off it.
        let (pa, pb) = if on[key.0 as usize] {
            (xz(verts[key.0 as usize]), xz(verts[key.1 as usize]))
        } else {
            (xz(verts[key.1 as usize]), xz(verts[key.0 as usize]))
        };
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if on_main(patch, add(pa, mul(sub(pb, pa), mid))) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let x = add(pa, mul(sub(pb, pa), lo));
        verts.push(at_y(x, patch.surface(x)));
        let i = (verts.len() - 1) as u32;
        cuts.insert(key, i);
        i
    };
    let mut out = Vec::with_capacity(tris.len() + 16);
    let mut out_sector = Vec::with_capacity(tris.len() + 16);
    for (t, k) in tris.into_iter().zip(sector) {
        let s = t.map(|i| on[i as usize]);
        if s[0] == s[1] && s[1] == s[2] {
            out.push(t);
            out_sector.push(k);
            continue;
        }
        // The corner alone on its side, then the other two in order.
        let lone = (0..3)
            .find(|&i| s[i] != s[(i + 1) % 3] && s[i] != s[(i + 2) % 3])
            .expect("one corner differs");
        let (a, b, c) = (t[lone], t[(lone + 1) % 3], t[(lone + 2) % 3]);
        let xab = cut(verts, a, b);
        let xac = cut(verts, a, c);
        for piece in [[a, xab, xac], [xab, b, c], [xab, c, xac]] {
            out.push(piece);
            out_sector.push(k);
        }
    }
    (out, out_sector)
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

/// What happens at a vertex (kept in an `Arc` per vertex: the size of a
/// patch beside a mitre costs nothing).
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum VertexRule {
    Patch(Patch),
    Mitre(Mitre),
}

/// How a strip's end is shaped.
#[derive(Debug, Clone, Copy)]
pub enum EndRule<'a> {
    Patch(&'a Patch, usize),
    Mitre(&'a Mitre, usize),
    /// The edge lies inside a cluster's patch (J3): no strip, the patch's
    /// surface all along.
    Inside(&'a Patch),
}

/// Edges at most this long between two junction vertices that do not both
/// get a patch alone join them into one cluster (J3: dual carriageways,
/// offset T junctions, slip roads a few metres apart).
pub const CLUSTER_EDGE_M: f64 = 12.0;
/// A cluster has at most this many vertices.
pub const CLUSTER_MAX: usize = 6;
/// A junction's arm runs on past a plain way split at most this far from
/// the junction (m, J8) …
pub const RUN_ON_M: f64 = 15.0;
/// … where the two edges bend by at most this much (degrees).
pub const RUN_ON_BEND_DEG: f64 = 20.0;
/// The mouths move out for their corners' radius along the real edge
/// lines at most this many times (J8).
pub const MOUTH_ROUNDS: usize = 3;
/// … on arms at most this steep at their mouth.
pub const GROW_MAX_GRADE: f64 = 0.06;

/// Per edge, its painted crossings: (distance from its `from` vertex, half
/// the painted length) (J9).
pub type Crossings = Vec<Vec<(f64, f64)>>;

/// The mouths of a vertex's patch: (edge, its `from` end there, mouth m).
type Mouths = Vec<(usize, bool, f64)>;

/// The junctions of a road graph, computed per vertex when first asked.
#[derive(Debug)]
pub struct JunctionSet {
    /// The edges at each vertex that take part in its junction: the
    /// paths drawn as a band of a road's side left out (J7, [`alongside`]).
    adjacency: Vec<Vec<usize>>,
    /// The number of edges at each vertex, those paths included: a vertex
    /// where they cross a street stays a member of its cluster (J7).
    degree: Vec<usize>,
    /// Per edge: a path drawn as a band of a road's side (J7).
    side: Arc<Vec<bool>>,
    rules: Vec<OnceLock<Option<Arc<VertexRule>>>>,
    /// Per vertex: the mouths of its patch alone (edge, its `from` end
    /// there, mouth m), `None` when it gets none (clusters, J3).
    alone: Vec<OnceLock<Option<Mouths>>>,
    /// The package's stored junctions (section `JNCT`, spec §4.8.8 J2):
    /// which vertices have a rule and the patches' outlines, so nothing is
    /// searched at load. The result is the same as computed.
    stored: Option<Arc<section::JunctionIndex>>,
    /// Per edge: the painted crossings on its strips (distance from its
    /// `from` vertex, half the painted length), where the world builder
    /// knows them (J9): its clusters and longer mouths never swallow one
    /// that the junctions' own patches leave on the strip.
    crossings: Option<Arc<Crossings>>,
}

impl Clone for JunctionSet {
    fn clone(&self) -> Self {
        Self {
            adjacency: self.adjacency.clone(),
            degree: self.degree.clone(),
            side: self.side.clone(),
            rules: self.rules.clone(),
            alone: self.alone.clone(),
            stored: self.stored.clone(),
            crossings: self.crossings.clone(),
        }
    }
}

impl JunctionSet {
    pub fn new(g: &RoadGraph) -> Self {
        let full = g.adjacency();
        let side = alongside::side_paths(g);
        let degree: Vec<usize> = full.iter().map(Vec::len).collect();
        let adjacency = full
            .into_iter()
            .map(|a| a.into_iter().filter(|&e| !side[e]).collect())
            .collect();
        Self {
            adjacency,
            degree,
            side: Arc::new(side),
            rules: (0..g.vertices.len()).map(|_| OnceLock::new()).collect(),
            alone: (0..g.vertices.len()).map(|_| OnceLock::new()).collect(),
            stored: None,
            crossings: None,
        }
    }

    /// The junctions of `g` from its package's stored index (section
    /// `JNCT`) when that was made for this graph (same fingerprint and
    /// vertex count), else computed as [`JunctionSet::new`].
    /// The junctions of `g` knowing the painted crossings of its edges
    /// (`per_edge[e]`: distance from the edge's `from` vertex, half the
    /// painted length; J9): a cluster's simple outline or an arm taking the
    /// room its far end leaves is not used where it would swallow more of
    /// them than the junctions' own patches (a crossing inside a junction
    /// surface is drawn at its mouth, not where OSM has it). Ignored when
    /// it does not have one list per edge.
    pub fn with_crossings(mut self, g: &RoadGraph, per_edge: Arc<Crossings>) -> Self {
        if per_edge.len() == g.edges.len() {
            self.crossings = Some(per_edge);
        }
        self
    }

    /// How many painted crossings patch `p` swallows: those on its arms'
    /// edges within their mouths and those on the edges inside it (J9).
    fn swallowed(&self, g: &RoadGraph, p: &Patch) -> usize {
        count_swallowed(self.crossings.as_deref(), g, p)
    }

    pub fn with_index(g: &RoadGraph, index: Option<Arc<section::JunctionIndex>>) -> Self {
        let mut set = Self::new(g);
        set.stored = index.filter(|i| i.fits(g));
        set
    }

    /// Per edge: a path drawn as a band of a road's side, not a road of
    /// its own (J7, [`alongside`]): it takes no part in the junctions.
    pub fn side_paths(&self) -> &[bool] {
        &self.side
    }

    /// Whether the rules come from a stored index.
    pub fn is_stored(&self) -> bool {
        self.stored.is_some()
    }

    /// The rule at vertex `v` of `g` (the graph this set was made for).
    pub fn rule(&self, g: &RoadGraph, v: usize) -> Option<&VertexRule> {
        self.rule_arc(g, v).map(|_| ())?;
        self.rules.get(v)?.get()?.as_deref()
    }

    fn rule_arc(&self, g: &RoadGraph, v: usize) -> Option<Arc<VertexRule>> {
        let slot = self.rules.get(v)?;
        slot.get_or_init(|| {
            if let Some(index) = &self.stored {
                let rule = match index.get(v as u32)? {
                    section::StoredRule::Mitre => {
                        let adj = &self.adjacency[v];
                        (adj.len() == 2)
                            .then(|| build_mitre(g, adj, v))
                            .flatten()
                            .filter(|r| matches!(r, VertexRule::Mitre(_)))
                    }
                    section::StoredRule::Patch(p) => stored_patch(g, v, p).map(VertexRule::Patch),
                    section::StoredRule::Member(first) => {
                        let first = *first as usize;
                        if first == v || first >= g.vertices.len() {
                            return None;
                        }
                        return self
                            .rule_arc(g, first)
                            .filter(|r| matches!(&**r, VertexRule::Patch(p) if p.members.contains(&(v as u32))));
                    }
                };
                return rule.map(Arc::new);
            }
            if !self.usable(g, v) {
                return None;
            }
            let adj = &self.adjacency[v];
            // A cluster (J3): made at its first member, shared by the
            // others; if it cannot be made, each keeps its own rule.
            if let Some((members, inner)) = self.cluster_of(g, v) {
                let first = members[0] as usize;
                if first == v {
                    if let Ok(p) = try_cluster(g, &self.adjacency, &members, &inner, self.crossings.as_deref()) {
                        return Some(Arc::new(VertexRule::Patch(p)));
                    }
                } else if let Some(r) = self.rule_arc(g, first)
                    && matches!(&*r, VertexRule::Patch(p) if p.members.contains(&(v as u32)))
                {
                    return Some(r);
                }
            }
            // A way split a junction's arm runs on past (J8): that
            // junction's patch.
            if adj.len() == 2
                && let Some(u) = self.run_on_owner(g, v)
                && let Some(r) = self.rule_arc(g, u)
                && matches!(&*r, VertexRule::Patch(p) if p.members.contains(&(v as u32)))
            {
                return Some(r);
            }
            let rule = if adj.len() < 2 {
                None
            } else if adj.len() == 2 {
                build_mitre(g, adj, v)
            } else {
                self.run_on_patch(g, v)
                    .or_else(|| self.room_patch(g, v))
                    .or_else(|| build_patch(g, adj, v))
                    .map(VertexRule::Patch)
            };
            rule.map(Arc::new)
        })
        .clone()
    }

    /// Whether vertex `v` may get a rule: 2 – [`MAX_ARMS`] edges, not at the
    /// region border, no bridge, tunnel or loop at it.
    fn usable(&self, g: &RoadGraph, v: usize) -> bool {
        let adj = &self.adjacency[v];
        // (Side paths counted: a vertex where a sidewalk way meets a short
        // link from the street may join a cluster, J7.)
        !(self.degree[v] < 2
            || adj.len() > MAX_ARMS
            || g.boundary_vertices.contains(&VertexId(v as u32))
            || adj.iter().any(|&e| {
                let e = &g.edges[e];
                e.bridge || e.tunnel || e.from == e.to
            }))
    }

    /// The mouths of junction vertex `v`'s patch alone (as J7 placed them,
    /// without J8's mouths out for the corners: the clusters and the way
    /// splits run on past are decided from them); `None`: it gets none.
    fn alone(&self, g: &RoadGraph, v: usize) -> Option<&[(usize, bool, f64)]> {
        self.alone[v]
            .get_or_init(|| {
                try_patch_with(g, &self.adjacency[v], v, false)
                    .ok()
                    .map(|p| {
                        p.arms
                            .iter()
                            .map(|a| (a.edge, a.at_from, a.mouth_m))
                            .collect()
                    })
            })
            .as_deref()
    }

    /// Whether the short edge `ei` from `u` to `w` joins them into a
    /// cluster: one of them gets no patch alone, or their patches meet on
    /// it (their mouths together cover 85 % of it: a small roundabout's
    /// ring, carriageways side by side).
    fn joins(&self, g: &RoadGraph, ei: usize, u: usize, w: usize) -> bool {
        // A vertex where only side paths make it a junction (J7) joins.
        if self.adjacency[u].len() < 3 || self.adjacency[w].len() < 3 {
            return true;
        }
        let (Some(a), Some(b)) = (self.alone(g, u), self.alone(g, w)) else {
            return true;
        };
        let e = &g.edges[ei];
        let mouth = |m: &[(usize, bool, f64)], from: bool| {
            m.iter()
                .find(|x| x.0 == ei && x.1 == from)
                .map_or(0.0, |x| x.2)
        };
        let (ma, mb) = (
            mouth(a, e.from.0 as usize == u),
            mouth(b, e.from.0 as usize == w),
        );
        ma + mb >= 0.85 * e.length_m()
    }

    /// The cluster of vertex `v` (J3): the junction vertices (3 or more
    /// edges) joined to it by edges of at most [`CLUSTER_EDGE_M`] on the
    /// ground whose two ends do not both get a patch alone, or whose
    /// patches nearly meet on them ([`Self::joins`]) – its members
    /// (growing) and those edges (growing); `None` for a vertex on its own
    /// or a cluster of more than [`CLUSTER_MAX`].
    pub fn cluster_of(&self, g: &RoadGraph, v: usize) -> Option<(Vec<u32>, Vec<usize>)> {
        // A junction: 3 or more edges, side paths counted (J7: where a
        // sidewalk way crosses a street beside a junction, that vertex
        // stays a member – the street's arm goes on from it; its own
        // drawn edges are 2). A cluster needs one junction of its own.
        let junction = |u: usize| self.usable(g, u) && self.degree[u] >= 3;
        if !(junction(v) && (self.adjacency[v].len() >= 3 || self.side_member(g, v))) {
            return None;
        }
        let mut members = std::collections::BTreeSet::from([v as u32]);
        let mut inner = std::collections::BTreeSet::new();
        let mut stack = vec![v];
        while let Some(u) = stack.pop() {
            for &ei in &self.adjacency[u] {
                let e = &g.edges[ei];
                let w = if e.from.0 as usize == u {
                    e.to.0
                } else {
                    e.from.0
                } as usize;
                if w == u
                    || self.side[ei]
                    || e.bridge
                    || e.tunnel
                    || e.profile.len() < 2
                    || e.length_m() > CLUSTER_EDGE_M
                    || !junction(w)
                    || !self.joins(g, ei, u, w)
                {
                    continue;
                }
                inner.insert(ei);
                if members.insert(w as u32) {
                    if members.len() > CLUSTER_MAX {
                        return None;
                    }
                    stack.push(w);
                }
            }
        }
        (members.len() > 1).then(|| (members.into_iter().collect(), inner.into_iter().collect()))
    }

    /// Whether vertex `v`, a junction only by side paths (J7), lies within
    /// [`CLUSTER_EDGE_M`] of a junction of its own along a drawn edge.
    fn side_member(&self, g: &RoadGraph, v: usize) -> bool {
        self.adjacency[v].iter().any(|&ei| {
            let e = &g.edges[ei];
            let w = e.other(VertexId(v as u32)).0 as usize;
            w != v
                && e.length_m() <= CLUSTER_EDGE_M
                && self.usable(g, w)
                && self.adjacency[w].len() >= 3
        })
    }

    /// A plain way split at the far end of edge `ei` from vertex `v` (J8):
    /// its other end `w`, where one other drawn edge goes on – of the same
    /// class and width, bending at most [`RUN_ON_BEND_DEG`], no side path,
    /// bridge, tunnel, loop or the region border there. `(w, the edge
    /// going on)`.
    pub fn split_beyond(&self, g: &RoadGraph, v: usize, ei: usize) -> Option<(usize, usize)> {
        let e = g.edges.get(ei)?;
        let w = e.other(VertexId(v as u32)).0 as usize;
        if w == v || self.degree[w] != 2 || self.adjacency[w].len() != 2 {
            return None;
        }
        let adj = &self.adjacency[w];
        let oi = if adj[0] == ei { adj[1] } else { adj[0] };
        if oi == ei || !adj.contains(&ei) {
            return None;
        }
        let o = &g.edges[oi];
        let far = o.other(VertexId(w as u32)).0 as usize;
        if far == v
            || far == w
            || g.boundary_vertices.contains(&VertexId(w as u32))
            || e.bridge
            || e.tunnel
            || o.bridge
            || o.tunnel
            || o.class != e.class
            || (o.width_m - e.width_m).abs() > WIDTH_CHANGE_M
            || o.profile.len() < 2
            || e.profile.len() < 2
        {
            return None;
        }
        let out_dir = |x: &Edge| {
            let pts = &x.points;
            if x.from.0 as usize == w {
                unit(sub(pts[1], pts[0]))
            } else {
                unit(sub(pts[pts.len() - 2], pts[pts.len() - 1]))
            }
        };
        let bend = std::f64::consts::PI - dot(out_dir(e), out_dir(o)).clamp(-1.0, 1.0).acos();
        (bend <= RUN_ON_BEND_DEG.to_radians()).then_some((w, oi))
    }

    /// A junction that may run its arms on past way splits (J8): 3 or more
    /// drawn edges, a rule of its own (not a cluster's).
    fn run_on_junction(&self, g: &RoadGraph, u: usize) -> bool {
        self.usable(g, u) && self.adjacency[u].len() >= 3 && self.cluster_of(g, u).is_none()
    }

    /// Whether junction `u`'s arm on edge `ei` is squeezed by its edge:
    /// its patch alone puts that mouth at its share of the edge
    /// ([`MOUTH_SHARE`]), or it gets no patch alone.
    fn squeezed(&self, g: &RoadGraph, u: usize, ei: usize) -> bool {
        let e = &g.edges[ei];
        let at_from = e.from.0 as usize == u;
        match self.alone(g, u) {
            None => true,
            Some(m) => m
                .iter()
                .find(|x| x.0 == ei && x.1 == at_from)
                .is_none_or(|x| x.2 >= MOUTH_SHARE * e.length_m() - 1e-6),
        }
    }

    /// Whether junction `u` runs its arm on edge `ei` on past the plain way
    /// split at the edge's far end (J8): the edge shorter than
    /// [`RUN_ON_M`], the arm squeezed by it, and the junction at the far
    /// end of the edge going on does not run on past the same split (the
    /// split belongs to neither then). `(the split, the edge going on)`.
    pub fn runs_on(&self, g: &RoadGraph, u: usize, ei: usize) -> Option<(usize, usize)> {
        let e = g.edges.get(ei)?;
        if e.length_m() >= RUN_ON_M || !self.run_on_junction(g, u) {
            return None;
        }
        let (w, oi) = self.split_beyond(g, u, ei)?;
        if !self.squeezed(g, u, ei) {
            return None;
        }
        let o = &g.edges[oi];
        let far = o.other(VertexId(w as u32)).0 as usize;
        if o.length_m() < RUN_ON_M
            && self.run_on_junction(g, far)
            && self.split_beyond(g, far, oi).is_some()
            && self.squeezed(g, far, oi)
        {
            return None;
        }
        Some((w, oi))
    }

    /// The junction whose arm runs on past way split `w` (J8).
    fn run_on_owner(&self, g: &RoadGraph, w: usize) -> Option<usize> {
        if self.adjacency[w].len() != 2 || self.degree[w] != 2 {
            return None;
        }
        self.adjacency[w].iter().find_map(|&ei| {
            let u = g.edges[ei].other(VertexId(w as u32)).0 as usize;
            (u != w && self.runs_on(g, u, ei).is_some_and(|(x, _)| x == w)).then_some(u)
        })
    }

    /// The patch of junction `v` with its arms run on past the way splits
    /// that squeeze them (J8); `None` where it has none to run on past or
    /// that patch cannot be made.
    fn run_on_patch(&self, g: &RoadGraph, v: usize) -> Option<Patch> {
        if !self.run_on_junction(g, v) {
            return None;
        }
        let adj = &self.adjacency[v];
        let runs: Vec<(usize, usize, usize)> = adj
            .iter()
            .filter_map(|&ei| self.runs_on(g, v, ei).map(|(w, oi)| (ei, w, oi)))
            .collect();
        if runs.is_empty() {
            return None;
        }
        let c = g.vertices[v];
        let mut raws = Vec::with_capacity(adj.len());
        for &ei in adj {
            let r = match runs.iter().find(|x| x.0 == ei) {
                Some(&(_, w, oi)) => raw_run_on(g, v, ei, w, oi),
                None => raw_arm(g, c, v, ei),
            };
            raws.push(r.ok()?);
        }
        let mut run_on: Vec<u32> = runs.iter().map(|x| x.1 as u32).collect();
        run_on.sort_unstable();
        let mut inner: Vec<usize> = runs.iter().map(|x| x.0).collect();
        inner.sort_unstable();
        let mut members = vec![v as u32];
        members.extend(run_on.iter().copied());
        let mut site = Site {
            centre: c,
            members,
            inner,
            run_on,
            grow: true,
            free: false,
        };
        // Where the mouths moved out for the corners cannot be made, as
        // they were; and only where its sides bend no more than the
        // junction's own patch's.
        let run_on = outline_checked(g, &site, raws.clone(), true)
            .and_then(|(patch, outline)| finish(patch, outline))
            .or_else(|_| {
                site.grow = false;
                let (patch, outline) = outline_checked(g, &site, raws, true)?;
                finish(patch, outline)
            })
            .ok()?;
        match try_patch(g, adj, v) {
            Ok(own) if side_bend(&run_on) > side_bend(&own) + 1e-3 => None,
            _ => Some(run_on),
        }
    }

    /// Why the cluster of vertex `v` could not be made (`None`: it has
    /// none, or it was made): diagnostics for `examples/side_dump` (J9).
    pub fn cluster_failure(&self, g: &RoadGraph, v: usize) -> Option<&'static str> {
        let (members, inner) = self.cluster_of(g, v)?;
        try_cluster(
            g,
            &self.adjacency,
            &members,
            &inner,
            self.crossings.as_deref(),
        )
        .err()
    }

    /// How far out junction `v`'s arm on edge `ei` may lie where the edge's
    /// far end leaves room (J9): an end with no junction surface – a
    /// bridge or tunnel, a dead end, the region border, a way split drawn
    /// as a mitre (its strip points within its shift dropped) – up to
    /// [`ROOM_MARGIN_M`] before what that end takes; only on arms at most
    /// [`GROW_MAX_GRADE`] steep there (a mouth farther out on a steeper
    /// one leaves its blend less room). `None`: its share of the edge.
    fn far_room(&self, g: &RoadGraph, v: usize, ei: usize) -> Option<f64> {
        let e = &g.edges[ei];
        let len = e.length_m();
        let at_from = e.from.0 as usize == v;
        let w = e.other(VertexId(v as u32)).0 as usize;
        if w == v || e.profile.len() < 2 {
            return None;
        }
        let share = MOUTH_SHARE * len;
        let s = if at_from { share } else { len - share };
        if grade(e, s, if at_from { 1.0 } else { -1.0 }).abs() > GROW_MAX_GRADE {
            return None;
        }
        let adj = &self.adjacency[w];
        let taken = if !self.usable(g, w) || adj.len() < 2 {
            0.0
        } else if adj.len() == 2 && self.degree[w] == 2 {
            if self.run_on_owner(g, w).is_some() {
                return None;
            }
            match build_mitre(g, adj, w) {
                Some(VertexRule::Mitre(m)) => m.shift_m + 0.3,
                _ => 0.0,
            }
        } else {
            return None;
        };
        let room = len - taken - ROOM_MARGIN_M;
        (room > share + 0.05).then_some(room)
    }

    /// The patch of junction `v` with its arms' mouths allowed out to the
    /// room their edges' far ends leave ([`Self::far_room`], J9); `None`
    /// where no arm has such room, the patch cannot be made, or its sides
    /// would bend more than those of its patch with the mouths at their
    /// shares (or that has none: no new surfaces).
    fn room_patch(&self, g: &RoadGraph, v: usize) -> Option<Patch> {
        if !self.run_on_junction(g, v) {
            return None;
        }
        let adj = &self.adjacency[v];
        let c = g.vertices[v];
        let mut raws = Vec::with_capacity(adj.len());
        let mut any = false;
        for &ei in adj {
            let mut r = raw_arm(g, c, v, ei).ok()?;
            r.room = self.far_room(g, v, ei);
            any |= r.room.is_some();
            raws.push(r);
        }
        if !any {
            return None;
        }
        let own = try_patch(g, adj, v).ok()?;
        let (patch, outline) = outline_checked(g, &Site::single(g, v), raws, false).ok()?;
        let room = finish(patch, outline).ok()?;
        (side_bend(&room) < side_bend(&own) - 1e-3
            && self.swallowed(g, &room) <= self.swallowed(g, &own))
        .then_some(room)
    }

    /// The patch of vertex `v` if it is drawn there (a cluster's at its
    /// first member only): every patch once.
    pub fn own_patch(&self, g: &RoadGraph, v: usize) -> Option<&Patch> {
        self.patch(g, v).filter(|p| p.is_at(v))
    }

    /// Why vertex `v` has no junction surface (`None`: it has one, or a
    /// mitre): diagnostics for the analysis test.
    pub fn fallback_reason(&self, g: &RoadGraph, v: usize) -> Option<&'static str> {
        let adj = &self.adjacency[v];
        if adj.len() < 2 {
            return Some("a dead end");
        }
        if adj.len() > MAX_ARMS {
            return Some("more than 8 arms");
        }
        if g.boundary_vertices.contains(&VertexId(v as u32)) {
            return Some("the region border");
        }
        if adj.iter().any(|&e| {
            let e = &g.edges[e];
            e.bridge || e.tunnel
        }) {
            return Some("a bridge or tunnel");
        }
        if adj.iter().any(|&e| g.edges[e].from == g.edges[e].to) {
            return Some("a loop edge");
        }
        if self.rule(g, v).is_some() {
            return None;
        }
        if adj.len() == 2 {
            return Some("a straight way split");
        }
        try_patch(g, adj, v).err()
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
                VertexRule::Patch(p) if p.inner.contains(&ei) => Some(EndRule::Inside(p)),
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
        let cum = cumulative(&g.edges[p.arms[arm].edge].points);
        Self::blend_row_on(g, p, arm, s, &cum)
    }

    /// [`Self::blend_row`] with the edge's cumulative lengths.
    fn blend_row_on(g: &RoadGraph, p: &Patch, arm: usize, s: f64, cum: &[f64]) -> [[f64; 3]; 2] {
        let a = &p.arms[arm];
        let e = &g.edges[a.edge];
        let (c, fwd) = frame_at(&e.points, cum, a.s_edge(s));
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

    /// How far past its mouth an arm's strip turns its cross slope flat
    /// (m, J5): [`Arm::warp_m`], within the room the edge leaves before its
    /// other end's junction (0: no cross slope, or a blended arm).
    pub fn warp_len(&self, g: &RoadGraph, p: &Patch, arm: usize) -> f64 {
        let a = &p.arms[arm];
        if a.cross == 0.0 || a.warp_m <= 0.0 || a.blend_m > 0.0 {
            return 0.0;
        }
        let ends = self.ends(g, a.edge);
        let other = ends[usize::from(a.at_from)]
            .as_ref()
            .map_or(0.0, |r| match r {
                EndRule::Patch(q, i) => q.arms[*i].mouth_m + q.arms[*i].blend_m,
                EndRule::Mitre(m, _) => m.shift_m + 0.3,
                EndRule::Inside(_) => a.len,
            });
        a.warp_m.min(a.len - a.mouth_m - other).max(0.0)
    }

    /// How far past its mouth an arm's strip has generated rows (m): its
    /// blend onto the main road (J2) or the turn of its cross slope to flat
    /// (J5); 0: none.
    pub fn past_m(&self, g: &RoadGraph, p: &Patch, arm: usize) -> f64 {
        let a = &p.arms[arm];
        if a.blend_m > 0.0 {
            a.blend_m
        } else {
            self.warp_len(g, p, arm)
        }
    }

    /// The distances from the vertex of the generated rows past an arm's
    /// mouth (the mouth first; empty: none).
    pub fn past_steps(&self, g: &RoadGraph, p: &Patch, arm: usize) -> Vec<f64> {
        let a = &p.arms[arm];
        let l = self.past_m(g, p, arm);
        if l <= 0.0 {
            return Vec::new();
        }
        let k = (l / BLEND_ROW_M).ceil().max(1.0) as usize;
        (0..=k)
            .map(|i| a.mouth_m + l * i as f64 / k as f64)
            .collect()
    }

    /// The strip's lean at `s` metres from edge `ei`'s `from` vertex (the
    /// rise of its right corner per metre of half width, in the edge's
    /// direction): what is left of the cross slopes its two ends' mouths
    /// lean by (J5).
    pub fn lean_at(&self, g: &RoadGraph, ei: usize, s: f64) -> f64 {
        let len = g.edges[ei].length_m();
        let mut lean = 0.0;
        for (k, end) in self.ends(g, ei).iter().enumerate() {
            let Some(EndRule::Patch(p, ai)) = end else {
                continue;
            };
            let a = &p.arms[*ai];
            let l = self.warp_len(g, p, *ai);
            if l <= 0.0 {
                continue;
            }
            let m = if k == 0 { s } else { len - s };
            let x = ((m - a.mouth_m) / l).clamp(0.0, 1.0);
            let left = 1.0 - x * x * (3.0 - 2.0 * x);
            lean += a.cross * left * if a.at_from { 1.0 } else { -1.0 };
        }
        lean
    }

    /// The generated row `m` metres from the vertex past arm `arm`'s mouth.
    pub fn past_row(&self, g: &RoadGraph, p: &Patch, arm: usize, m: f64) -> [[f64; 3]; 2] {
        let cum = cumulative(&g.edges[p.arms[arm].edge].points);
        self.past_row_on(g, p, arm, m, &cum)
    }

    fn past_row_on(
        &self,
        g: &RoadGraph,
        p: &Patch,
        arm: usize,
        m: f64,
        cum: &[f64],
    ) -> [[f64; 3]; 2] {
        let a = &p.arms[arm];
        if a.blend_m > 0.0 {
            return Self::blend_row_on(g, p, arm, m, cum);
        }
        // Leaning by what is left of the cross slopes, flat after.
        let e = &g.edges[a.edge];
        let s = a.s_edge(m);
        let (c, fwd) = frame_at(&e.points, cum, s);
        let right = side_pos(fwd);
        let h = height(e, s);
        let lean = self.lean_at(g, a.edge, s);
        [
            at_y(sub(c, mul(right, a.half)), h - lean * a.half),
            at_y(add(c, mul(right, a.half)), h + lean * a.half),
        ]
    }

    /// The drawn junction surface at `p` on edge `ei`, `s` metres from its
    /// `from` vertex: inside a patch its mesh, along a blended side arm its
    /// rows; `None` elsewhere (the road's own profile applies).
    pub fn drawn_height(&self, g: &RoadGraph, ei: usize, s: f64, p: P2) -> Option<f64> {
        self.drawn_height_from(g, ei, s, p, [true, true])
    }

    /// [`Self::drawn_height`] from the junction at end `k` of edge `ei` only
    /// (0 its `from` end, 1 its `to` end).
    pub fn drawn_height_at_end(
        &self,
        g: &RoadGraph,
        ei: usize,
        k: usize,
        s: f64,
        p: P2,
    ) -> Option<f64> {
        self.drawn_height_from(g, ei, s, p, [k == 0, k == 1])
    }

    fn drawn_height_from(
        &self,
        g: &RoadGraph,
        ei: usize,
        s: f64,
        p: P2,
        which: [bool; 2],
    ) -> Option<f64> {
        let ends = self.ends(g, ei);
        let len = g.edges[ei].length_m();
        for (k, end) in ends.iter().enumerate() {
            if !which[k] {
                continue;
            }
            if let Some(EndRule::Inside(patch)) = end {
                return patch.height_at(p).or_else(|| Some(patch.field(p)));
            }
            let Some(EndRule::Patch(patch, ai)) = end else {
                continue;
            };
            let a = &patch.arms[*ai];
            let m = if k == 0 { s } else { len - s };
            if m <= a.mouth_m + 1e-9 {
                return patch.height_at(p).or_else(|| Some(patch.field(p)));
            }
            let past = self.past_m(g, patch, *ai);
            if past > 0.0 && m <= a.mouth_m + past {
                let steps = self.past_steps(g, patch, *ai);
                let i = steps.partition_point(|&x| x <= m).clamp(1, steps.len() - 1);
                let cum = cumulative(&g.edges[ei].points);
                let r0 = self.past_row_on(g, patch, *ai, steps[i - 1], &cum);
                let r1 = self.past_row_on(g, patch, *ai, steps[i], &cum);
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
                let r = Self::blend_row_on(g, p, arm, s, &cum);
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
        let len = g.edges[ei].length_m();
        self.ends(g, ei).map(|e| match e {
            Some(EndRule::Patch(p, i)) => p.arms[i].mouth_m + p.arms[i].blend_m,
            Some(EndRule::Inside(_)) => len,
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
    for (k, w) in e.points.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
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
        // Far from both ends' junctions (straight-line distance, never more
        // than along the road): nothing to do. Most pieces of long roads.
        let e = &g.edges[ei];
        let ends = self.ends(g, ei);
        let near = |k: usize, v: VertexId| -> bool {
            let reach = match ends[k] {
                Some(EndRule::Patch(p, i)) => p.arms[i].mouth_m + self.past_m(g, p, i) + 1.0,
                Some(EndRule::Mitre(m, _)) => m.shift_m + 1.0,
                Some(EndRule::Inside(_)) => return true,
                None => return false,
            };
            let c = g.vertices[v.0 as usize];
            pts.iter()
                .any(|q| (f64::from(q[0]) - c[0]).hypot(f64::from(q[2]) - c[1]) <= reach)
        };
        if !near(0, e.from) && !near(1, e.to) {
            return None;
        }
        if !strip_reversed(e, pts) {
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
        // Inside a cluster's patch: no strip.
        if ends.iter().any(|e| matches!(e, Some(EndRule::Inside(_)))) {
            return Some(Shaped::default());
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
        let mut near_mouth: [Option<f64>; 2] = [None, None];
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
                    let past = self.past_m(g, p, *ai);
                    if past <= 0.0 {
                        // No kept point right past the mouth: its row,
                        // square to the strip there, would fold back over
                        // the mouth row where the road bends (J5).
                        near_mouth[k] = Some(m_edge);
                    }
                    if past > 0.0 {
                        let steps = self.past_steps(g, p, *ai);
                        let (b0, b1) = (a.s_edge(steps[0]), a.s_edge(*steps.last().unwrap()));
                        replaced.push((b0.min(b1), b0.max(b1)));
                        // Only the rows on this piece of the strip.
                        let on = |se: f64| se >= s_first - 1e-6 && se <= s_last + 1e-6;
                        for &st in &steps[1..] {
                            let se = a.s_edge(st);
                            if on(se) {
                                generated.push((se, self.past_row_on(g, p, *ai, st, &cum)));
                            }
                        }
                        // The strip piece may start or end inside the blend.
                        for se in [s_first, s_last] {
                            let m = if k == 0 { se } else { len - se };
                            if m > a.mouth_m + 1e-6 && m < a.mouth_m + past - 1e-6 {
                                generated.push((se, self.past_row_on(g, p, *ai, m, &cum)));
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
                // Handled before (no strip).
                Some(EndRule::Inside(_)) | None => {}
            }
        }
        if s_last <= lo + 1e-6 || s_first >= hi - 1e-6 {
            return Some(Shaped::default());
        }
        // Kept plain points.
        let n = pts.len();
        // (edge metres, centre, explicit row).
        type Item = (f64, [f32; 3], Option<[[f32; 3]; 2]>);
        let mut items: Vec<Item> = Vec::new();
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
            if near_mouth
                .iter()
                .flatten()
                .any(|&m| (si - m).abs() < MOUTH_CLEAR_M)
                && i > 0
                && i + 1 < n
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
        // §4.8.8 Q7: a side road's strip gets a row every 0.5 m wherever
        // it comes near the main road's carriageway (past its mouth, up to
        // `MAIN_REACH_M` on), so it can lie under the main road's surface
        // exactly as far as it reaches onto it (below), also where the main
        // road curves under a long quad.
        for end in &ends {
            let Some(EndRule::Patch(p, ai)) = end else {
                continue;
            };
            if !p.through.is_some_and(|t| !t.contains(ai)) {
                continue;
            }
            let a = &p.arms[*ai];
            let near = |c: [f32; 3], d: P2| {
                let c = [f64::from(c[0]), f64::from(c[2])];
                let r = side_pos(d);
                [-1.0, 0.0, 1.0].into_iter().any(|k| {
                    let q = add(c, mul(r, k * f64::from(half)));
                    p.main_road_height(g, q, MAIN_REACH_M, SIDE_MARGIN_M)
                        .is_some()
                })
            };
            let mut extra = Vec::new();
            for w in items.windows(2) {
                let ((s0, c0, _), (s1, c1, _)) = (w[0], w[1]);
                let m0 = if a.at_from { s0 } else { len - s0 };
                if (s1 - s0).abs() < 0.6 || m0 > a.mouth_m + MAIN_REACH_M {
                    continue;
                }
                let d = unit([f64::from(c1[0] - c0[0]), f64::from(c1[2] - c0[2])]);
                let steps = ((s1 - s0).abs() / 0.5).floor() as usize;
                let at = |t: f64| -> [f32; 3] {
                    std::array::from_fn(|i| c0[i] + (c1[i] - c0[i]) * t as f32)
                };
                if !(0..=steps).any(|q| near(at(q as f64 / steps as f64), d)) {
                    continue;
                }
                for q in 1..steps {
                    let t = q as f64 / steps as f64;
                    extra.push((s0 + (s1 - s0) * t, at(t), None));
                }
            }
            items.extend(extra);
        }
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut points: Vec<[f32; 3]> = items.iter().map(|it| it.1).collect();
        let mut rows: Vec<[[f32; 3]; 2]> = items
            .iter()
            .enumerate()
            .map(|(i, it)| it.2.unwrap_or_else(|| plain_row(&points, i, half)))
            .collect();
        // No row whose corner lies behind the row before it (on the inner
        // side of a sharp bend, rows square to different pieces of the
        // centre line cross): the strip's edge would fold back (J5, §4.8.8
        // Rule). The two end rows stay.
        let mut keep = vec![true; rows.len()];
        let mut last = 0;
        for i in 1..rows.len() {
            let (a, b) = (rows[last], rows[i]);
            let ahead = (0..2).all(|k| {
                let d = [b[k][0] - a[k][0], b[k][2] - a[k][2]];
                let c = [
                    points[i][0] - points[last][0],
                    points[i][2] - points[last][2],
                ];
                d[0] * c[0] + d[1] * c[1] > 0.0
            });
            if ahead || i + 1 == rows.len() {
                last = i;
            } else {
                keep[i] = false;
            }
        }
        if keep.iter().any(|k| !k) {
            let mut k = keep.iter();
            items.retain(|_| *k.next().expect("same length"));
            let mut k = keep.iter();
            points.retain(|_| *k.next().expect("same length"));
            let mut k = keep.iter();
            rows.retain(|_| *k.next().expect("same length"));
        }
        // §4.8.8 Q7: a side road's strip never stands above (or on) the
        // main road's carriageway where it crosses it past its mouth (a
        // narrow side road whose mouth lies on the main road): there it
        // lies under the main road's surface, which is drawn.
        for end in &ends {
            let Some(EndRule::Patch(p, ai)) = end else {
                continue;
            };
            if !p.through.is_some_and(|t| !t.contains(ai)) {
                continue;
            }
            let mouth = p.arms[*ai].s_edge(p.arms[*ai].mouth_m);
            let main_at = |q: &[f32; 3]| {
                p.main_road_height(
                    g,
                    [f64::from(q[0]), f64::from(q[2])],
                    MAIN_REACH_M,
                    SIDE_MARGIN_M,
                )
            };
            let on = |q: &[f32; 3]| main_at(q).is_some();
            let under = |q: &mut [f32; 3]| {
                let x = [f64::from(q[0]), f64::from(q[2])];
                if let Some(m) = main_at(q).or_else(|| p.main_own(x)) {
                    let cap = (m - SIDE_UNDER_M) as f32;
                    if q[1] > cap {
                        q[1] = cap;
                    }
                }
            };
            // A row with a corner on the carriageway lies under it whole (a
            // row across its edge would rise above it between its corners).
            for (i, it) in items.iter().enumerate() {
                if (it.0 - mouth).abs() < 1e-6 || !rows[i].iter().any(on) {
                    continue;
                }
                rows[i].iter_mut().for_each(under);
                under(&mut points[i]);
            }
            // J6 (§4.8.8 Rule: a side road never above the main road): the
            // drawn triangles between two rows, sampled, wherever they lie
            // over the main road's real carriageway (its curving centre
            // line) – a strip leaving at a slant or along a curving main
            // road reaches over it between rows whose corners lie beside
            // it. Such rows go down until the triangles lie under it; the
            // mouth row stays (the patch's), the row after it goes down by
            // as much more as its share there asks.
            let s_mouth = mouth;
            let room = |s: f64| {
                let m = if p.arms[*ai].at_from { s } else { len - s };
                m <= p.arms[*ai].mouth_m + MAIN_REACH_M
            };
            for _ in 0..4 {
                let mut changed = false;
                for i in 0..rows.len().saturating_sub(1) {
                    if !room(items[i].0) && !room(items[i + 1].0) {
                        continue;
                    }
                    let fixed = [
                        (items[i].0 - s_mouth).abs() < 1e-6,
                        (items[i + 1].0 - s_mouth).abs() < 1e-6,
                    ];
                    let [l0, r0] = rows[i];
                    let [l1, r1] = rows[i + 1];
                    let mut need = [0.0f64; 2];
                    // The two triangles as drawn ([l0, r0, l1], [r0, r1, l1]),
                    // each corner's row.
                    for (tri, of) in [([l0, r0, l1], [0, 0, 1]), ([r0, r1, l1], [0, 1, 1])] {
                        for a in 0..=4 {
                            for b in 0..=(4 - a) {
                                let w = [a as f64 / 4.0, b as f64 / 4.0, (4 - a - b) as f64 / 4.0];
                                let at = |k: usize| -> f64 {
                                    (0..3).map(|c| w[c] * f64::from(tri[c][k])).sum()
                                };
                                let q = [at(0), at(2)];
                                let Some(m) = p.main_road_height(g, q, MAIN_REACH_M, 0.0) else {
                                    continue;
                                };
                                let over = at(1) - m;
                                if over <= 0.005 {
                                    continue;
                                }
                                let drop = over + SIDE_UNDER_M;
                                // The share of each row in this point.
                                let share = [
                                    (0..3).filter(|&c| of[c] == 0).map(|c| w[c]).sum::<f64>(),
                                    (0..3).filter(|&c| of[c] == 1).map(|c| w[c]).sum::<f64>(),
                                ];
                                match fixed {
                                    [true, false] => {
                                        need[1] = need[1].max(drop / share[1].max(0.25));
                                    }
                                    [false, true] => {
                                        need[0] = need[0].max(drop / share[0].max(0.25));
                                    }
                                    [false, false] => {
                                        need = [need[0].max(drop), need[1].max(drop)];
                                    }
                                    [true, true] => {}
                                }
                            }
                        }
                    }
                    for (k, d) in need.into_iter().enumerate() {
                        // At most half a metre a round (a strip that would
                        // need more stands steep over the main road).
                        let d = d.min(0.5);
                        if d > 0.0 {
                            let r = i + k;
                            for c in &mut rows[r] {
                                c[1] -= d as f32;
                            }
                            points[r][1] -= d as f32;
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        }
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

pub mod alongside;
pub mod fixtures;
mod plane_mesh;
pub mod section;
mod sides;

#[cfg(test)]
mod tests;

/// TEMPORARY (J5 diagnostics): the J5 chain of `patch` from arm `i` to the
/// next, and whether it keeps the outline star-shaped.
#[doc(hidden)]
pub fn debug_chain(
    g: &RoadGraph,
    patch: &Patch,
    i: usize,
) -> (Option<Vec<P2>>, bool, [Vec<P2>; 2]) {
    let j = (i + 1) % patch.arms.len();
    let (a, b) = (&patch.arms[i], &patch.arms[j]);
    let lines = [sides::edge_line(g, a, 1.0), sides::edge_line(g, b, -1.0)];
    let c = xz(patch.centre);
    let ch = sides::corner_chain(
        g,
        a,
        b,
        corner_radius(a.class.max(b.class)),
        (patch.members.len() > 1 && a.origin != b.origin).then_some(c),
        [false, false],
    );
    let ok = ch
        .as_ref()
        .is_some_and(|c| sides::polar_ok(xz(patch.centre), c));
    (ch, ok, lines)
}
