//! Low-poly mesh building for the generated assets: tubes, sweeps, lofts,
//! ellipsoids, tori, boxes and discs, collected per paint slot.

use std::collections::BTreeMap;
use std::f32::consts::TAU;

use glam::{Quat, Vec3};

use crate::model::{Model, Primitive};
use crate::slots;

/// Triangles in one material, in model space while being built.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
    /// Vertex colours (linear RGBA, [`Primitive::colors`]) of the first
    /// `colors.len()` vertices; the others are white. Empty: none. The
    /// faces use them (spec 14 Phase 4, 4N *Faces*): a shade on skin, eyes
    /// and lips, the hair's density on brows, moustaches and beards.
    pub colors: Vec<[f32; 4]>,
}

/// One ring of a [`Shape::loft`]: the points `centre + u cos t + v sin t`.
#[derive(Debug, Clone, Copy)]
pub struct Ring {
    pub centre: Vec3,
    pub u: Vec3,
    pub v: Vec3,
}

impl Ring {
    /// A circle of radius `r` around `axis` (any orientation of `u`).
    pub fn circle(centre: Vec3, axis: Vec3, r: f32) -> Self {
        let d = axis.normalize();
        let u = d.any_orthonormal_vector();
        Self {
            centre,
            u: u * r,
            v: d.cross(u) * r,
        }
    }

    /// An ellipse in the plane ⟂ `axis` with half-axes `a` along `side`
    /// (made ⟂ `axis`) and `b` across.
    pub fn ellipse(centre: Vec3, axis: Vec3, side: Vec3, a: f32, b: f32) -> Self {
        let d = axis.normalize();
        let u = (side - d * side.dot(d)).normalize();
        Self {
            centre,
            u: u * a,
            v: d.cross(u) * b,
        }
    }

    fn point(&self, t: f32) -> Vec3 {
        self.centre + self.u * t.cos() + self.v * t.sin()
    }
}

/// How a loft's open end is closed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cap {
    Open,
    /// A flat disc (its own flat normals).
    Flat,
    /// A cone to this point (smooth with the sides: domes, poles).
    Point(Vec3),
}

impl Shape {
    fn base(&self) -> u32 {
        self.positions.len() as u32
    }

    /// Gives every vertex added since `from` (a [`Shape::positions`]
    /// length) the colour `f` returns for its index from there.
    pub fn colour_from(&mut self, from: usize, f: impl Fn(usize) -> [f32; 4]) {
        self.colors.resize(from, [1.0; 4]);
        self.colors
            .extend((0..self.positions.len() - from).map(f));
    }

    /// The vertex colours for a primitive: none, or one per vertex.
    pub fn colours(&self) -> Vec<[f32; 4]> {
        if self.colors.is_empty() {
            return Vec::new();
        }
        let mut c = self.colors.clone();
        c.resize(self.positions.len(), [1.0; 4]);
        c
    }

    /// Adds triangles with area-weighted smooth vertex normals.
    pub fn add_smooth(&mut self, verts: &[Vec3], tris: &[[u32; 3]]) {
        let base = self.base();
        let mut normals = vec![Vec3::ZERO; verts.len()];
        for t in tris {
            let [a, b, c] = t.map(|i| verts[i as usize]);
            let n = (b - a).cross(c - a);
            for &i in t {
                normals[i as usize] += n;
            }
        }
        self.positions.extend_from_slice(verts);
        self.normals
            .extend(normals.into_iter().map(|n| n.normalize_or(Vec3::Y)));
        self.indices
            .extend(tris.iter().flat_map(|t| t.map(|i| base + i)));
    }

    /// Adds triangles with one normal per face (vertices are duplicated).
    pub fn add_flat(&mut self, verts: &[Vec3], tris: &[[u32; 3]]) {
        for t in tris {
            let [a, b, c] = t.map(|i| verts[i as usize]);
            let n = (b - a).cross(c - a).normalize_or(Vec3::Y);
            let base = self.base();
            self.positions.extend([a, b, c]);
            self.normals.extend([n; 3]);
            self.indices.extend([base, base + 1, base + 2]);
        }
    }

    /// A surface through `rings` (each with `sides` points), outward
    /// facing; `closed` joins the last ring to the first (a torus).
    pub fn loft(&mut self, rings: &[Ring], sides: u32, closed: bool, start: Cap, end: Cap) {
        assert!(rings.len() >= 2 && sides >= 3);
        // Outward faces need u × v along the loft's direction.
        let dir = rings[1].centre - rings[0].centre;
        let flip = rings[0].u.cross(rings[0].v).dot(dir) < 0.0;
        let rings: Vec<Ring> = rings
            .iter()
            .map(|r| Ring {
                v: if flip { -r.v } else { r.v },
                ..*r
            })
            .collect();
        let n = sides as usize;
        let mut verts: Vec<Vec3> = Vec::with_capacity(rings.len() * n + 2);
        for r in &rings {
            for k in 0..n {
                verts.push(r.point(k as f32 / n as f32 * TAU));
            }
        }
        let mut tris = Vec::new();
        let count = rings.len();
        let segs = if closed { count } else { count - 1 };
        let at = |i: usize, k: usize| ((i % count) * n + k % n) as u32;
        for i in 0..segs {
            for k in 0..n {
                let (a, b, c, d) = (at(i, k), at(i, k + 1), at(i + 1, k), at(i + 1, k + 1));
                tris.push([a, b, c]);
                tris.push([b, d, c]);
            }
        }
        if !closed {
            for (cap, ring, outward_first) in [(start, 0, true), (end, count - 1, false)] {
                match cap {
                    Cap::Open => {}
                    Cap::Point(p) => {
                        let apex = verts.len() as u32;
                        verts.push(p);
                        for k in 0..n {
                            let (a, b) = (at(ring, k), at(ring, k + 1));
                            tris.push(if outward_first {
                                [apex, b, a]
                            } else {
                                [apex, a, b]
                            });
                        }
                    }
                    Cap::Flat => {
                        let r = &rings[ring];
                        let mut fan = vec![r.centre];
                        fan.extend((0..n).map(|k| verts[at(ring, k) as usize]));
                        let ft: Vec<[u32; 3]> = (0..n as u32)
                            .map(|k| {
                                let (a, b) = (1 + k, 1 + (k + 1) % n as u32);
                                if outward_first { [0, b, a] } else { [0, a, b] }
                            })
                            .collect();
                        self.add_flat(&fan, &ft);
                    }
                }
            }
        }
        self.add_smooth(&verts, &tris);
    }

    /// A (tapered) round tube from `a` (radius `ra`) to `b` (radius `rb`).
    pub fn tube(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, sides: u32, caps: bool) {
        let d = b - a;
        let r0 = Ring::circle(a, d, ra);
        let s = rb / ra.max(1e-6);
        let r1 = Ring {
            centre: b,
            u: r0.u * s,
            v: r0.v * s,
        };
        let cap = if caps { Cap::Flat } else { Cap::Open };
        self.loft(&[r0, r1], sides, false, cap, cap);
    }

    /// A tube along a polyline with a radius per point (parallel-transported
    /// rings, no twist).
    pub fn sweep(&mut self, path: &[Vec3], radii: &[f32], sides: u32, caps: bool) {
        assert!(path.len() >= 2 && path.len() == radii.len());
        let tangent = |i: usize| {
            let prev = path[i.saturating_sub(1)];
            let next = path[(i + 1).min(path.len() - 1)];
            (next - prev).normalize()
        };
        let mut t0 = tangent(0);
        let mut u = t0.any_orthonormal_vector();
        let mut rings = Vec::with_capacity(path.len());
        for i in 0..path.len() {
            let t = tangent(i);
            // Parallel transport: turn u by the rotation from t0 to t.
            u = Quat::from_rotation_arc(t0, t) * u;
            u = (u - t * u.dot(t)).normalize();
            t0 = t;
            rings.push(Ring {
                centre: path[i],
                u: u * radii[i],
                v: t.cross(u) * radii[i],
            });
        }
        let cap = if caps { Cap::Flat } else { Cap::Open };
        self.loft(&rings, sides, false, cap, cap);
    }

    /// An ellipsoid with half-axes `radii` (x, y, z before `rot`).
    pub fn ellipsoid(&mut self, centre: Vec3, radii: Vec3, rot: Quat, bands: u32, sides: u32) {
        let rings: Vec<Ring> = (1..bands)
            .map(|j| {
                let a = std::f32::consts::PI * j as f32 / bands as f32;
                let (s, c) = (a.sin(), -a.cos());
                Ring {
                    centre: centre + rot * Vec3::new(0.0, c * radii.y, 0.0),
                    u: rot * Vec3::new(s * radii.x, 0.0, 0.0),
                    v: rot * Vec3::new(0.0, 0.0, -s * radii.z),
                }
            })
            .collect();
        let bottom = centre + rot * Vec3::new(0.0, -radii.y, 0.0);
        let top = centre + rot * Vec3::new(0.0, radii.y, 0.0);
        self.loft(&rings, sides, false, Cap::Point(bottom), Cap::Point(top));
    }

    /// A torus around `axis` (a tyre): tube radius `r` on a circle of
    /// radius `major`; `knobs` > 0 makes every other ring that much
    /// fatter outwards (tread).
    #[allow(clippy::too_many_arguments)]
    pub fn torus(
        &mut self,
        centre: Vec3,
        axis: Vec3,
        major: f32,
        r: f32,
        segments: u32,
        sides: u32,
        knobs: f32,
    ) {
        let a = axis.normalize();
        let e0 = a.any_orthonormal_vector();
        let rings: Vec<Ring> = (0..segments)
            .map(|i| {
                let ang = i as f32 / segments as f32 * TAU;
                let e = Quat::from_axis_angle(a, ang) * e0;
                let k = if knobs > 0.0 && i % 2 == 1 {
                    1.0 + knobs
                } else {
                    1.0
                };
                Ring {
                    centre: centre + e * major,
                    u: e * r * k,
                    v: -a * r,
                }
            })
            .collect();
        self.loft(&rings, sides, true, Cap::Open, Cap::Open);
    }

    /// A box with half extents `half`, turned by `rot` (flat faces).
    pub fn cuboid(&mut self, centre: Vec3, half: Vec3, rot: Quat) {
        let c = |x: f32, y: f32, z: f32| centre + rot * (half * Vec3::new(x, y, z));
        let v = [
            c(-1., -1., -1.),
            c(1., -1., -1.),
            c(1., 1., -1.),
            c(-1., 1., -1.),
            c(-1., -1., 1.),
            c(1., -1., 1.),
            c(1., 1., 1.),
            c(-1., 1., 1.),
        ];
        let quads = [
            [0, 3, 2, 1], // -z
            [4, 5, 6, 7], // +z
            [0, 4, 7, 3], // -x
            [1, 2, 6, 5], // +x
            [0, 1, 5, 4], // -y
            [3, 7, 6, 2], // +y
        ];
        let tris: Vec<[u32; 3]> = quads
            .iter()
            .flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]])
            .collect();
        self.add_flat(&v, &tris);
    }

    /// A flat ring (chainring, brake rotor; a disc when `inner` is 0)
    /// around `axis`, `thickness` thick.
    pub fn annulus(
        &mut self,
        centre: Vec3,
        axis: Vec3,
        inner: f32,
        outer: f32,
        thickness: f32,
        segments: u32,
    ) {
        let a = axis.normalize() * thickness * 0.5;
        if inner <= 0.0 {
            self.tube(centre - a, centre + a, outer, outer, segments, true);
            return;
        }
        let o0 = Ring::circle(centre - a, a, outer);
        let o1 = Ring {
            centre: centre + a,
            ..o0
        };
        self.loft(&[o0, o1], segments, false, Cap::Open, Cap::Open);
        // Inner wall: facing the axis (rings ordered against the axis).
        let s = inner / outer;
        let i0 = Ring {
            centre: centre + a,
            u: o0.u * s,
            v: o0.v * s,
        };
        let i1 = Ring {
            centre: centre - a,
            ..i0
        };
        // Built facing outward, then flipped to face inward.
        let start = self.indices.len();
        self.loft(&[i0, i1], segments, false, Cap::Open, Cap::Open);
        for t in self.indices[start..].chunks_mut(3) {
            t.swap(1, 2);
        }
        let first_vertex = self.indices[start..].iter().min().copied().unwrap_or(0) as usize;
        for n in &mut self.normals[first_vertex..] {
            *n = -*n;
        }
        // The two faces.
        for (side, sign) in [(centre + a, 1.0f32), (centre - a, -1.0)] {
            let n = segments as usize;
            let mut verts = Vec::with_capacity(2 * n);
            for k in 0..n {
                let t = k as f32 / n as f32 * TAU;
                let dir = (o0.u * t.cos() + o0.v * t.sin()) / outer;
                verts.push(side + dir * outer);
                verts.push(side + dir * inner);
            }
            let mut tris = Vec::new();
            for k in 0..n as u32 {
                let (o, i, o2, i2) = (
                    2 * k,
                    2 * k + 1,
                    2 * ((k + 1) % n as u32),
                    2 * ((k + 1) % n as u32) + 1,
                );
                if sign > 0.0 {
                    tris.push([o, o2, i]);
                    tris.push([i, o2, i2]);
                } else {
                    tris.push([o, i, o2]);
                    tris.push([i, i2, o2]);
                }
            }
            self.add_flat(&verts, &tris);
        }
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// The triangles' corner positions.
    pub fn tris(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|i| self.positions[i as usize]))
    }

    /// Every vertex normal again from its triangles, each weighted by the
    /// triangle's angle at the vertex – the way Blender computes a
    /// smooth vertex normal. A custom normal Blender would compute
    /// itself is stored as "its own" and comes back exactly; one far
    /// from it is stored relative to it and can come back up to 0.8°
    /// off (an angle snapped to its reference edge; spec 15P, the lower
    /// lip of Phase 15 Q11). Use it on small smooth parts whose
    /// area-weighted normals Blender rewrites.
    pub fn smooth_by_angle(&mut self) {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for t in self.indices.as_chunks::<3>().0 {
            let p = t.map(|i| self.positions[i as usize]);
            let face = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            for k in 0..3 {
                let (a, b) = (p[(k + 1) % 3] - p[k], p[(k + 2) % 3] - p[k]);
                normals[t[k] as usize] += face * a.angle_between(b);
            }
        }
        for (n, m) in self.normals.iter_mut().zip(normals) {
            *n = m.normalize_or(*n);
        }
    }
}

/// Where the ray from `o` along `d` meets triangle `t`: the ray's
/// parameter (> 0; the distance when `d` is a unit vector), or `None`.
pub fn ray_triangle(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let h = d.cross(e2);
    let a = e1.dot(h);
    if a.abs() < 1e-12 {
        return None;
    }
    let f = 1.0 / a;
    let s = o - t[0];
    let u = f * s.dot(h);
    let q = s.cross(e1);
    let v = f * d.dot(q);
    let k = f * e2.dot(q);
    ((0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 && k > 0.0).then_some(k)
}

/// How far the ray from `o` along `d` reaches before it leaves the
/// (closed or open) surface `tris` for good: its last crossing, or `None`
/// when it misses.
pub fn ray_last_hit(o: Vec3, d: Vec3, tris: &[[Vec3; 3]]) -> Option<f32> {
    tris.iter()
        .filter_map(|t| ray_triangle(o, d, t))
        .fold(None, |m: Option<f32>, k| Some(m.map_or(k, |m| m.max(k))))
}

/// Shapes by paint slot, for one node.
#[derive(Debug, Clone, Default)]
pub struct Parts {
    by_slot: BTreeMap<&'static str, Shape>,
}

impl Parts {
    pub fn new() -> Self {
        Self::default()
    }

    /// The shape of paint slot `slot` (one of [`slots::ALL`]).
    pub fn slot(&mut self, slot: &'static str) -> &mut Shape {
        debug_assert!(slots::get(slot).is_some(), "unknown slot {slot}");
        self.by_slot.entry(slot).or_default()
    }

    pub fn triangles(&self) -> usize {
        self.by_slot.values().map(Shape::triangles).sum()
    }

    /// The shapes with their paint slots.
    pub fn shapes(&self) -> impl Iterator<Item = (&'static str, &Shape)> {
        self.by_slot.iter().map(|(slot, shape)| (*slot, shape))
    }

    /// The same shapes with their slots mapped by `f` (shapes that land
    /// on one slot are merged: fewer draw calls).
    pub fn remap(self, f: impl Fn(&str) -> &'static str) -> Parts {
        let mut out = Parts::new();
        for (slot, shape) in self.by_slot {
            let to = out.slot(f(slot));
            let base = to.positions.len() as u32;
            if !shape.colors.is_empty() || !to.colors.is_empty() {
                to.colors.resize(base as usize, [1.0; 4]);
                to.colors.extend(shape.colours());
            }
            to.positions.extend(shape.positions);
            to.normals.extend(shape.normals);
            to.indices
                .extend(shape.indices.into_iter().map(|i| base + i));
        }
        out
    }

    /// Moves the shapes (built in model space) into node `node` of `model`:
    /// into the node's local frame, one primitive per slot.
    pub fn attach(self, model: &mut Model, node: usize) {
        let inv = model.world(node).inverse();
        for (slot, shape) in self.by_slot {
            if shape.indices.is_empty() {
                continue;
            }
            let material = model.material_or_add(slots::material(slot));
            let colors = shape.colours();
            model.nodes[node].primitives.push(Primitive {
                material,
                colors,
                positions: shape
                    .positions
                    .iter()
                    .map(|p| inv.transform_point3(*p).to_array())
                    .collect(),
                normals: shape
                    .normals
                    .iter()
                    .map(|n| inv.transform_vector3(*n).normalize().to_array())
                    .collect(),
                indices: shape.indices,
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every face of a closed convex shape points away from its centre.
    fn outward(s: &Shape, centre: Vec3) {
        for t in s.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| s.positions[t[k] as usize]);
            let n = (b - a).cross(c - a);
            let mid = (a + b + c) / 3.0;
            assert!(n.dot(mid - centre) > -1e-7, "inward face at {mid}");
        }
        for (i, n) in s.normals.iter().enumerate() {
            assert!((n.length() - 1.0).abs() < 1e-4, "normal {i} {n}");
        }
    }

    #[test]
    fn closed_shapes_face_outwards() {
        let mut s = Shape::default();
        s.tube(Vec3::ZERO, Vec3::new(0.2, 1.0, -0.3), 0.1, 0.05, 8, true);
        outward(&s, Vec3::new(0.1, 0.5, -0.15));
        let mut s = Shape::default();
        s.ellipsoid(
            Vec3::ONE,
            Vec3::new(0.1, 0.2, 0.3),
            Quat::from_rotation_x(0.4),
            6,
            8,
        );
        outward(&s, Vec3::ONE);
        // The pole caps share the sides' smooth normals.
        assert!(s.normals.iter().all(|n| n.is_finite()));
        let mut s = Shape::default();
        s.cuboid(
            Vec3::ZERO,
            Vec3::new(0.1, 0.2, 0.3),
            Quat::from_rotation_y(1.0),
        );
        outward(&s, Vec3::ZERO);
        assert_eq!(s.triangles(), 12);
        let mut s = Shape::default();
        s.annulus(Vec3::ZERO, Vec3::X, 0.0, 0.1, 0.01, 12);
        outward(&s, Vec3::ZERO);
    }

    /// `smooth_by_angle` weights each triangle's normal by its corner
    /// angle (a vertex of a thin sliver gets little of the sliver's
    /// normal) and keeps the shape.
    #[test]
    fn angle_weighted_normals() {
        let mut s = Shape::default();
        s.ellipsoid(
            Vec3::ZERO,
            Vec3::new(0.0125, 0.0036, 0.0032),
            Quat::IDENTITY,
            3,
            6,
        );
        let pos = s.positions.clone();
        // A fan around one vertex: a wide triangle and a thin sliver.
        let mut fan = Shape::default();
        let v = [
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(0.0, 1.0, 1.0).normalize(),
            Vec3::new(0.05, 1.0, 0.0).normalize(),
        ];
        fan.add_smooth(&v, &[[0, 1, 2], [0, 2, 3]]);
        fan.smooth_by_angle();
        assert_eq!(fan.positions, v);
        let f0 = (v[1]).cross(v[2]).normalize();
        let f1 = (v[2]).cross(v[3]).normalize();
        let (a0, a1) = (v[1].angle_between(v[2]), v[2].angle_between(v[3]));
        let want = (f0 * a0 + f1 * a1).normalize();
        assert!(fan.normals[0].distance(want) < 1e-5, "{}", fan.normals[0]);
        let mut again = s.clone();
        again.smooth_by_angle();
        assert_eq!(again.positions, pos);
        assert!(
            again
                .normals
                .iter()
                .all(|n| (n.length() - 1.0).abs() < 1e-5)
        );
        // The rider's mouth inside (a coarse flat ellipsoid): weighted by
        // area some normals lie behind their triangle, by angle none.
        let behind = |s: &Shape| {
            s.indices.chunks(3).any(|t| {
                let [a, b, c] = [0, 1, 2].map(|k| s.positions[t[k] as usize]);
                let face = (b - a).cross(c - a).normalize();
                t.iter().any(|&k| s.normals[k as usize].dot(face) <= 0.05)
            })
        };
        assert!(behind(&s), "area-weighted");
        assert!(!behind(&again), "angle-weighted");
    }

    #[test]
    fn a_torus_faces_away_from_its_tube_centre() {
        let mut s = Shape::default();
        let (major, axis) = (0.3, Vec3::X);
        s.torus(Vec3::ZERO, axis, major, 0.02, 24, 6, 0.0);
        for t in s.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| s.positions[t[k] as usize]);
            let mid = (a + b + c) / 3.0;
            let radial = (mid - axis * mid.dot(axis)).normalize();
            let tube_centre = radial * major;
            assert!((b - a).cross(c - a).dot(mid - tube_centre) > 0.0);
        }
        assert_eq!(s.triangles(), 24 * 6 * 2);
    }

    #[test]
    fn a_flat_ring_faces_outwards_and_inwards() {
        let mut s = Shape::default();
        s.annulus(Vec3::ZERO, Vec3::X, 0.05, 0.1, 0.02, 16);
        for t in s.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| s.positions[t[k] as usize]);
            let n = (b - a).cross(c - a).normalize();
            let mid = (a + b + c) / 3.0;
            let radial = Vec3::new(0.0, mid.y, mid.z);
            if n.x.abs() > 0.99 {
                assert!(n.x * mid.x > 0.0, "face {mid} {n}");
            } else if radial.length() > 0.075 {
                assert!(n.dot(radial) > 0.0, "outer wall {mid} {n}");
            } else {
                assert!(n.dot(radial) < 0.0, "inner wall {mid} {n}");
            }
        }
        // Stored normals agree with the winding.
        for t in s.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| s.positions[t[k] as usize]);
            let n = (b - a).cross(c - a);
            for k in t {
                assert!(s.normals[*k as usize].dot(n) > 0.0);
            }
        }
    }

    #[test]
    fn sweeps_follow_their_path() {
        let path = [
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, -1.0, -1.5),
        ];
        let mut s = Shape::default();
        s.sweep(&path, &[0.02, 0.02, 0.01], 6, false);
        // Every vertex lies within its ring's radius of the path.
        for p in &s.positions {
            let d = path
                .windows(2)
                .map(|w| {
                    let (a, b) = (w[0], w[1]);
                    let t = ((*p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                    (a + (b - a) * t).distance(*p)
                })
                .fold(f32::INFINITY, f32::min);
            assert!(d <= 0.0201, "{p} {d}");
        }
    }
}
