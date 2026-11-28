//! Holes in the drawn road surface, found by probing the road meshes from
//! above (spec 4.4 *Road surface without holes*; bug 2026-11-28: "in
//! andorra, sometimes i see holes in the street … so i can see through").
//!
//! The road meshes of a set of tiles (`road_mesh`, `junction_mesh`: every
//! triangle the renderer draws with the road material) go into a
//! [`SurfaceIndex`]. [`probe_strips`] then samples each road strip on a
//! fine grid inside its carriageway (along every segment, in a fan round
//! every bend on the outside, and in a half disc past a strip end where
//! another strip goes on) and asks the index, from straight above, for
//! the drawn surface at the strip's height:
//! - **covered**: an up-facing triangle within [`HEIGHT_TOL_M`];
//! - **inverted**: only a triangle whose winding faces down (the renderer
//!   culls back faces, so it is not drawn: a hole);
//! - **gap**: nothing there.
//!
//! Holes are grouped into [`Spot`]s (samples within [`SPOT_M`]) with their
//! cause class, for the analysis test and the regression tests.

use std::collections::HashMap;

use bevy::math::{Vec2, Vec3, Vec3Swizzles};
use ocw_world::package::RoadStrip;

use crate::package::MeshArrays;

/// Grid cell of the index (m).
const CELL_M: f32 = 2.0;
/// A triangle counts as the road's surface within this height of the
/// strip's drawn height (m).
pub const HEIGHT_TOL_M: f32 = 1.0;
/// Samples this far apart along and across a strip (m).
pub const STEP_M: f32 = 0.25;
/// Samples keep this far inside the carriageway's edges (m).
pub const EDGE_MARGIN_M: f32 = 0.2;
/// Hole samples nearer than this make one spot (m).
pub const SPOT_M: f32 = 3.0;
/// Samples keep this far inside a strip's own ends (m).
const END_INSET_M: f32 = 0.02;
/// Strip ends nearer than this are joined (m).
const JOINT_M: f32 = 0.05;

/// The triangles of the road meshes, by grid cell.
#[derive(Default)]
pub struct SurfaceIndex {
    tris: Vec<[Vec3; 3]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

fn cell(v: f32) -> i32 {
    (v / CELL_M).floor() as i32
}

/// What lies under a sample point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Probe {
    Covered,
    /// Only down-facing triangles (culled: a hole).
    Inverted,
    /// No triangle at all.
    Gap,
}

impl SurfaceIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Adds every triangle of `m`.
    pub fn add(&mut self, m: &MeshArrays) {
        for t in m.indices.as_chunks::<3>().0 {
            let v = [0, 1, 2].map(|k| Vec3::from(m.positions[t[k] as usize]));
            let i = self.tris.len() as u32;
            self.tris.push(v);
            let (lo, hi) = (v[0].min(v[1]).min(v[2]), v[0].max(v[1]).max(v[2]));
            for cx in cell(lo.x)..=cell(hi.x) {
                for cz in cell(lo.z)..=cell(hi.z) {
                    self.cells.entry((cx, cz)).or_default().push(i);
                }
            }
        }
    }

    /// The drawn surface at `p` (horizontal) near height `y`.
    pub fn probe(&self, p: Vec2, y: f32) -> Probe {
        self.probe_near(p, y, HEIGHT_TOL_M).0
    }

    /// [`Self::probe`] with the nearest up-facing surface at any height
    /// there (its height less `y`; the analysis: a road drawn higher or
    /// lower than its strip).
    pub fn probe_near(&self, p: Vec2, y: f32, tol: f32) -> (Probe, Option<f32>) {
        let Some(c) = self.cells.get(&(cell(p.x), cell(p.y))) else {
            return (Probe::Gap, None);
        };
        let mut back = false;
        let mut near: Option<f32> = None;
        for &i in c {
            let t = &self.tris[i as usize];
            let Some((h, up)) = tri_at(t, p) else {
                continue;
            };
            if (h - y).abs() <= tol {
                if up {
                    return (Probe::Covered, Some(h - y));
                }
                back = true;
            } else if up && near.is_none_or(|d| (h - y).abs() < d.abs()) {
                near = Some(h - y);
            }
        }
        (if back { Probe::Inverted } else { Probe::Gap }, near)
    }
}

/// A drawn road surface that can be probed: one [`SurfaceIndex`] or
/// several (a tile and its neighbours, [`Indexes`]).
pub trait Surface {
    /// What lies at `p` near height `y`, and the nearest up-facing surface
    /// at any height there (its height less `y`).
    fn probe_near(&self, p: Vec2, y: f32, tol: f32) -> (Probe, Option<f32>);
}

impl Surface for SurfaceIndex {
    fn probe_near(&self, p: Vec2, y: f32, tol: f32) -> (Probe, Option<f32>) {
        SurfaceIndex::probe_near(self, p, y, tol)
    }
}

/// Several indexes probed as one.
pub struct Indexes<'a>(pub Vec<&'a SurfaceIndex>);

impl Surface for Indexes<'_> {
    fn probe_near(&self, p: Vec2, y: f32, tol: f32) -> (Probe, Option<f32>) {
        let mut best = (Probe::Gap, None::<f32>);
        for i in &self.0 {
            let (probe, d) = i.probe_near(p, y, tol);
            if probe == Probe::Covered {
                return (probe, d);
            }
            best.0 = best.0.min(probe);
            if let Some(d) = d
                && best.1.is_none_or(|e| d.abs() < e.abs())
            {
                best.1 = Some(d);
            }
        }
        best
    }
}

/// Height of triangle `t` over `p` and whether it faces up (its winding
/// counter-clockwise seen from above), if `p` lies in it.
pub fn tri_at(t: &[Vec3; 3], p: Vec2) -> Option<(f32, bool)> {
    let (a, b, c) = (t[0].xz(), t[1].xz(), t[2].xz());
    // Seen from above (−y) with x right, z is down the screen: the
    // renderer's front faces have (b − a) × (c − a) pointing up (+y),
    // i.e. a negative 2D cross product in (x, z).
    let d = (b - a).perp_dot(c - a);
    if d.abs() < 1e-7 {
        return None;
    }
    let w1 = (p - a).perp_dot(c - a) / d;
    let w2 = (b - a).perp_dot(p - a) / d;
    let w0 = 1.0 - w1 - w2;
    let e = -1e-5;
    (w0 >= e && w1 >= e && w2 >= e).then(|| (w0 * t[0].y + w1 * t[1].y + w2 * t[2].y, d < 0.0))
}

/// Where a hole sample lies on its strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Place {
    /// Along a segment.
    Segment,
    /// In the fan on the outside of a bend.
    Bend,
    /// Past a strip's end where other strips go on: `n` other ends there
    /// (1: the same road, its next strip; more: a junction).
    Joint(u8),
}

/// One sample under no drawn road surface.
#[derive(Debug, Clone, Copy)]
pub struct HoleSample {
    pub strip: usize,
    pub at: Vec3,
    pub probe: Probe,
    pub place: Place,
    /// Signed distance from the strip's centreline (+ right; m).
    pub across: f32,
    /// Smallest radius of the strip's centreline near the sample (m).
    pub radius_m: f32,
    /// The nearest drawn surface above (+) or below there, if any (m).
    pub dy: Option<f32>,
}

/// The sample points of one strip with their place: (point with the
/// strip's height, place, across, radius).
fn strip_samples(
    s: &RoadStrip,
    joints: &[Vec<(Vec2, Vec2, f32)>; 2],
    mut f: impl FnMut(Vec3, Place, f32, f32, f32),
) {
    let p: Vec<Vec3> = s.points.iter().map(|q| Vec3::from(*q)).collect();
    let n = p.len();
    if n < 2 {
        return;
    }
    let half = s.width_m / 2.0 - EDGE_MARGIN_M;
    if half <= 0.0 {
        return;
    }
    let lift = crate::ROAD_SURFACE_LIFT_M;
    // Turning radius at each vertex (from its two segments).
    let radius: Vec<f32> = (0..n)
        .map(|i| {
            if i == 0 || i == n - 1 {
                return f32::INFINITY;
            }
            let (a, b, c) = (p[i - 1].xz(), p[i].xz(), p[i + 1].xz());
            let (u, v) = ((b - a), (c - b));
            let ang = u.angle_to(v).abs();
            if ang < 1e-4 {
                f32::INFINITY
            } else {
                (u.length().min(v.length())) / (2.0 * (ang / 2.0).tan())
            }
        })
        .collect();
    let across_steps = (2.0 * half / STEP_M).ceil().max(1.0) as i32;
    for i in 0..n - 1 {
        let (a, b) = (p[i], p[i + 1]);
        let d = (b - a).xz();
        let len = d.length();
        if len < 1e-4 {
            continue;
        }
        let dir = d / len;
        let right = Vec2::new(-dir.y, dir.x);
        let r = radius[i].min(radius[i + 1]);
        // Round a bend tighter than the road is wide the inner part of the
        // road is reached from points of different heights: the height
        // there is the strip's within its grade over the offset.
        let grade = (b.y - a.y).abs() / len;
        let loose = if r < 2.0 * s.width_m { grade } else { 0.0 };
        let along_steps = (len / STEP_M).ceil().max(1.0) as i32;
        // A strip's own ends a little inside (the mesh ends exactly there:
        // a sample on its border may fall either way in f32).
        let t0 = if i == 0 { END_INSET_M / len } else { 0.0 };
        let t1 = if i == n - 2 {
            1.0 - END_INSET_M / len
        } else {
            1.0
        };
        for k in 0..=along_steps {
            let t = t0 + (t1 - t0) * k as f32 / along_steps as f32;
            let c = a.lerp(b, t);
            for j in 0..=across_steps {
                let u = -half + 2.0 * half * j as f32 / across_steps as f32;
                let q = c.xz() + right * u;
                let tol = HEIGHT_TOL_M + loose * u.abs();
                f(Vec3::new(q.x, c.y + lift, q.y), Place::Segment, u, r, tol);
            }
        }
    }
    // The outside of every bend: between the two segments' normals.
    for i in 1..n - 1 {
        let d0 = (p[i] - p[i - 1]).xz().normalize_or_zero();
        let d1 = (p[i + 1] - p[i]).xz().normalize_or_zero();
        let turn = d0.perp_dot(d1);
        if turn.abs() < 1e-3 {
            continue;
        }
        // Turning right (turn > 0 with z down): the outside is the left.
        let side = if turn > 0.0 { -1.0 } else { 1.0 };
        let n0 = Vec2::new(-d0.y, d0.x) * side;
        let n1 = Vec2::new(-d1.y, d1.x) * side;
        fan(p[i] + Vec3::Y * lift, n0, n1, half, |q| {
            f(q, Place::Bend, side * half, radius[i], HEIGHT_TOL_M);
        });
    }
    // Past the ends where other strips go on.
    for (e, js) in joints.iter().enumerate() {
        if js.is_empty() {
            continue;
        }
        let (end, inner) = if e == 0 {
            (p[0], p[1])
        } else {
            (p[n - 1], p[n - 2])
        };
        let out = (end - inner).xz().normalize_or_zero();
        let right = Vec2::new(-out.y, out.x);
        let place = Place::Joint(js.len().min(255) as u8);
        // As far as the narrower of this road and the one going on
        // straightest from it reaches (a wide road ending on a narrow one
        // has no round end; at a junction without a surface the roads
        // leaving to the sides do not round this one's end).
        let other = js
            .iter()
            .min_by(|a, b| out.dot(a.1).total_cmp(&out.dot(b.1)))
            .map_or(0.0, |j| j.2);
        let r = half.min(other / 2.0 - EDGE_MARGIN_M);
        if r <= 0.0 {
            continue;
        }
        fan(end + Vec3::Y * lift, right, -right, r, |q| {
            f(q, place, 0.0, f32::INFINITY, HEIGHT_TOL_M);
        });
    }
}

/// Samples of a half disc (radius `r`) round `c` from direction `a` over
/// to `b` (the shorter way round through their bisector, or through the
/// outward side when they are opposite).
fn fan(c: Vec3, a: Vec2, b: Vec2, r: f32, mut f: impl FnMut(Vec3)) {
    let mut span = a.angle_to(b);
    if (span.abs() - std::f32::consts::PI).abs() < 1e-3 {
        // Opposite: sweep through the side `a` rotated a quarter turn
        // counter-clockwise in (x, z) (the end's outward direction).
        span = -std::f32::consts::PI;
    }
    let rings = (r / STEP_M).ceil().max(1.0) as i32;
    for k in 1..=rings {
        let rr = r * k as f32 / rings as f32;
        let arc = (rr * span.abs() / STEP_M).ceil().max(1.0) as i32;
        for j in 0..=arc {
            let ang = span * j as f32 / arc as f32;
            let d = Vec2::from_angle(ang).rotate(a);
            let q = c.xz() + d * rr;
            f(Vec3::new(q.x, c.y, q.y));
        }
    }
}

/// The strips' ends that meet other strips' ends: per strip, per end
/// [start, end], the (point, outward direction, width) of the others there.
fn joints(strips: &[RoadStrip]) -> Vec<[Vec<(Vec2, Vec2, f32)>; 2]> {
    let ends: Vec<[(Vec3, Vec2); 2]> = strips
        .iter()
        .map(|s| {
            let p = &s.points;
            let n = p.len();
            let at = |i: usize, j: usize| {
                let (a, b) = (Vec3::from(p[i]), Vec3::from(p[j]));
                (a, (a - b).xz().normalize_or_zero())
            };
            if n < 2 {
                [(Vec3::NAN, Vec2::ZERO); 2]
            } else {
                [at(0, 1), at(n - 1, n - 2)]
            }
        })
        .collect();
    let mut grid: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    for (i, e) in ends.iter().enumerate() {
        for (k, (p, _)) in e.iter().enumerate() {
            if p.is_finite() {
                grid.entry((cell(p.x), cell(p.z))).or_default().push((i, k));
            }
        }
    }
    let mut out = vec![[Vec::new(), Vec::new()]; strips.len()];
    for (i, e) in ends.iter().enumerate() {
        for (k, (p, _)) in e.iter().enumerate() {
            if !p.is_finite() {
                continue;
            }
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let Some(v) = grid.get(&(cell(p.x) + dx, cell(p.z) + dz)) else {
                        continue;
                    };
                    for &(j, l) in v {
                        if j == i {
                            continue;
                        }
                        let (q, d) = ends[j][l];
                        if q.xz().distance(p.xz()) < JOINT_M && (q.y - p.y).abs() < 1.0 {
                            out[i][k].push((q.xz(), d, strips[j].width_m));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Every sample of `strips` the drawn surface `index` does not cover;
/// `keep` limits the samples (e.g. to a tile, or away from a package's
/// border).
pub fn probe_strips(
    strips: &[RoadStrip],
    index: &dyn Surface,
    keep: &dyn Fn(Vec3) -> bool,
) -> (usize, Vec<HoleSample>) {
    let js = joints(strips);
    let mut total = 0;
    let mut holes = Vec::new();
    for (i, s) in strips.iter().enumerate() {
        strip_samples(s, &js[i], |q, place, across, radius_m, tol| {
            if !keep(q) {
                return;
            }
            total += 1;
            let (probe, dy) = index.probe_near(q.xz(), q.y, tol);
            if probe != Probe::Covered {
                holes.push(HoleSample {
                    strip: i,
                    at: q,
                    probe,
                    place,
                    across,
                    radius_m,
                    dy,
                });
            }
        });
    }
    (total, holes)
}

/// Hole samples near each other, with their worst probe and place.
#[derive(Debug, Clone)]
pub struct Spot {
    pub at: Vec3,
    pub samples: usize,
    pub probe: Probe,
    pub place: Place,
    pub strip: usize,
    pub radius_m: f32,
    /// The first sample's nearest drawn surface (m).
    pub dy: Option<f32>,
    /// The first sample's distance from the centreline (m).
    pub across: f32,
}

impl Spot {
    /// The area of the spot's samples (m²).
    pub fn area_m2(&self) -> f32 {
        self.samples as f32 * STEP_M * STEP_M
    }
}

/// Groups hole samples into spots ([`SPOT_M`]); the largest first.
pub fn spots(holes: &[HoleSample]) -> Vec<Spot> {
    let mut out: Vec<Spot> = Vec::new();
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    let g = |v: f32| (v / SPOT_M).floor() as i32;
    for h in holes {
        let (cx, cz) = (g(h.at.x), g(h.at.z));
        let mut hit = None;
        'find: for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(v) = grid.get(&(cx + dx, cz + dz)) {
                    for &si in v {
                        if out[si].at.distance(h.at) < SPOT_M {
                            hit = Some(si);
                            break 'find;
                        }
                    }
                }
            }
        }
        match hit {
            Some(si) => {
                let s = &mut out[si];
                s.samples += 1;
                s.probe = s.probe.min(h.probe);
                s.radius_m = s.radius_m.min(h.radius_m);
            }
            None => {
                grid.entry((cx, cz)).or_default().push(out.len());
                out.push(Spot {
                    at: h.at,
                    samples: 1,
                    probe: h.probe,
                    place: h.place,
                    strip: h.strip,
                    radius_m: h.radius_m,
                    dy: h.dy,
                    across: h.across,
                });
            }
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.samples));
    out
}

/// Area of the triangles of `m` that face down (culled: not drawn) among
/// those of road surface kind `kinds` (`road_mesh::Kind` codes), in m²
/// seen from above.
pub fn inverted_area_m2(m: &MeshArrays, kinds: &[u8]) -> f32 {
    let mut area = 0.0;
    for t in m.indices.as_chunks::<3>().0 {
        let k = crate::road_mesh::decode_b(m.uvs_b[t[0] as usize][0]).1;
        if !kinds.contains(&k) {
            continue;
        }
        let v = [0, 1, 2].map(|k| Vec3::from(m.positions[t[k] as usize]).xz());
        let d = (v[1] - v[0]).perp_dot(v[2] - v[0]);
        if d > 0.0 {
            area += d / 2.0;
        }
    }
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_faces_of_a_strip_face_up() {
        // road_mesh's carriageway winding: [l0, r0, l1] along +x (right +z).
        let t = [
            Vec3::new(0.0, 0.0, -2.0),
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(1.0, 0.0, -2.0),
        ];
        assert_eq!(tri_at(&t, Vec2::new(0.2, -1.0)), Some((0.0, true)));
        let back = [t[0], t[2], t[1]];
        assert_eq!(tri_at(&back, Vec2::new(0.2, -1.0)), Some((0.0, false)));
        assert_eq!(tri_at(&t, Vec2::new(2.0, 0.0)), None);
    }
}
