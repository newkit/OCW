//! House meshes (spec §14 Phase 8 T4): walls with procedural façades,
//! roof shapes (flat, gabled, hipped, half-hipped, pyramidal, skillion,
//! mansard, gambrel), eaves and parapets, in three levels of detail.
//!
//! Each building is resolved to a [`HouseSpec`] (`house_style`) and drawn
//! with the building material (`building.wesl`): every face carries its
//! material layer(s) and façade code in the second texture coordinate,
//! (along, up) in metres in the first, its colour (linear) in the vertex
//! colour and the door's bay in the colour's alpha:
//!
//! - `uv_b.x = lower layer + 16 × upper layer` (the upper layer above the
//!   first storey: timber upper floors);
//! - `uv_b.y = window + 16 × (storey height in dm + 64 × storeys)`, the
//!   window style + 1 or 0 (no windows: roofs, gables, the far LOD).
//!
//! **Levels of detail** (Bevy `VisibilityRange`, spec §4.5): *near* – per
//! [`NEAR_CHUNK_M`] chunk, up to [`NEAR_END_M`]: overhanging eaves and
//! verges with a roof slab, parapets, chimneys, canopy posts; *mid* – the
//! whole tile merged, up to [`MID_END_M`]: walls and roofs without
//! overhangs, set [`MID_INSET_M`] inside the near mesh; *far* – the whole
//! tile merged from [`FAR_START_M`] on: no windows, no buildings under
//! [`FAR_MIN_AREA_M2`], [`FAR_INSET_M`] inside. Coarser levels sit inside
//! the finer ones, so where two are drawn the finer one hides the other
//! (no z-fighting, no gaps, independent origins).

use bevy::math::{Vec2, Vec3};
use ocw_world::buildings::{Building, RoofShape, triangulate};

use crate::house_style::{HouseSpec, Layer, MAX_WINDOW_STYLES, WindowStyle};
use crate::package::MeshArrays;

/// Size of the near-detail chunks (m).
pub const NEAR_CHUNK_M: f32 = 250.0;
/// The near meshes fade out between these distances from their chunk's centre.
pub const NEAR_END_M: [f32; 2] = [330.0, 400.0];
/// The mid meshes fade out between these distances from the tile centre.
pub const MID_END_M: [f32; 2] = [1900.0, 2000.0];
/// The far meshes appear at this distance from the tile centre.
pub const FAR_START_M: f32 = 1300.0;
/// Mid walls and roofs sit this far inside the near ones.
pub const MID_INSET_M: f32 = 0.15;
/// Far walls and roofs sit this far inside.
pub const FAR_INSET_M: f32 = 0.4;
/// Smaller buildings are left out far away (m²).
pub const FAR_MIN_AREA_M2: f32 = 30.0;
/// Roof slab thickness in the near LOD (m).
pub const ROOF_SLAB_M: f32 = 0.18;
/// Gable verges overhang this share of the eaves' overhang.
pub const VERGE_SHARE: f32 = 0.7;

/// Level of detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lod {
    Near,
    Mid,
    Far,
}

impl Lod {
    fn inset(self) -> f32 {
        match self {
            Lod::Near => 0.0,
            Lod::Mid => MID_INSET_M,
            Lod::Far => FAR_INSET_M,
        }
    }
}

/// How a face is painted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paint {
    pub lower: Layer,
    pub upper: Layer,
    /// Linear RGB of the lower layer.
    pub colour: [f32; 3],
    /// Window style + 1 (0: none).
    pub window: u32,
    pub storey_dm: u32,
    pub levels: u32,
    /// Door bay + 1 on this wall (0: none).
    pub door: u32,
}

impl Paint {
    pub fn plain(layer: Layer, colour: [f32; 3]) -> Self {
        Self {
            lower: layer,
            upper: layer,
            colour,
            window: 0,
            storey_dm: 0,
            levels: 0,
            door: 0,
        }
    }

    pub fn uv_b(&self) -> [f32; 2] {
        let code = |l: Layer| f32::from(l as u8);
        // Storeys always (the upper floors switch material by them), the
        // window style only where windows are drawn.
        let facade = self.window + 16 * (self.storey_dm.min(63) + 64 * self.levels.min(255));
        [code(self.lower) + 16.0 * code(self.upper), facade as f32]
    }

    pub fn rgba(&self) -> [f32; 4] {
        [
            self.colour[0],
            self.colour[1],
            self.colour[2],
            self.door as f32,
        ]
    }
}

/// Decodes [`Paint::uv_b`] (the shader's twin): (lower, upper, window,
/// storey height m, storeys).
pub fn decode_uv_b(uv_b: [f32; 2]) -> (u32, u32, u32, f32, u32) {
    let l = uv_b[0].round() as u32;
    let f = uv_b[1].round() as u32;
    (
        l % 16,
        l / 16,
        f % 16,
        ((f / 16) % 64) as f32 / 10.0,
        f / (16 * 64),
    )
}

/// Texture coordinates of a face: `uv = (dot(p − at, u), dot(p − at, v) + v0)`.
#[derive(Debug, Clone, Copy)]
struct UvFrame {
    at: Vec3,
    u: Vec3,
    v: Vec3,
    v0: f32,
}

impl UvFrame {
    fn of(&self, p: Vec3) -> [f32; 2] {
        let d = p - self.at;
        [d.dot(self.u), d.dot(self.v) + self.v0]
    }

    /// A sloped or vertical plane: u along the horizontal `along`, v up the
    /// plane (metres along it).
    fn plane(at: Vec3, along: Vec3, normal: Vec3) -> Self {
        let u = along.with_y(0.0).normalize_or_zero();
        let mut v = normal.cross(u).normalize_or_zero();
        if v.y < 0.0 {
            v = -v;
        }
        Self { at, u, v, v0: 0.0 }
    }

    /// Flat: world x, z.
    fn flat() -> Self {
        Self {
            at: Vec3::ZERO,
            u: Vec3::X,
            v: Vec3::Z,
            v0: 0.0,
        }
    }
}

/// Mesh writer relative to an origin.
struct Out<'a> {
    m: &'a mut MeshArrays,
    origin: Vec3,
}

impl Out<'_> {
    /// A convex polygon (fan), turned to face `outward`.
    fn face(&mut self, poly: &[Vec3], outward: Vec3, paint: &Paint, uv: &UvFrame) {
        // Repeated points (a hip that became a pyramid) would leave slivers.
        let mut pts: Vec<Vec3> = Vec::with_capacity(poly.len());
        for p in poly {
            if pts.last().is_none_or(|q| q.distance_squared(*p) > 1e-10) {
                pts.push(*p);
            }
        }
        while pts.len() > 1 && pts[0].distance_squared(pts[pts.len() - 1]) <= 1e-10 {
            pts.pop();
        }
        let poly = pts.as_slice();
        if poly.len() < 3 {
            return;
        }
        let mut n = Vec3::ZERO;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            n += Vec3::new(
                (a.y - b.y) * (a.z + b.z),
                (a.z - b.z) * (a.x + b.x),
                (a.x - b.x) * (a.y + b.y),
            );
        }
        let n = n.normalize_or_zero();
        if n == Vec3::ZERO {
            return;
        }
        let flip = n.dot(outward) < 0.0;
        let normal = if flip { -n } else { n };
        let base = self.m.positions.len() as u32;
        for p in poly {
            self.m.positions.push((*p - self.origin).to_array());
            self.m.normals.push(normal.to_array());
            self.m.colors.push(paint.rgba());
            self.m.uvs.push(uv.of(*p));
            self.m.uvs_b.push(paint.uv_b());
        }
        for i in 1..poly.len() as u32 - 1 {
            if flip {
                self.m.indices.extend([base, base + i + 1, base + i]);
            } else {
                self.m.indices.extend([base, base + i, base + i + 1]);
            }
        }
    }

    /// Triangles by index into `pts` (a flat roof), facing up or down.
    fn tris(&mut self, pts: &[Vec3], tris: &[[u32; 3]], up: bool, paint: &Paint) {
        let normal = if up { Vec3::Y } else { Vec3::NEG_Y };
        let uv = UvFrame::flat();
        for t in tris {
            let base = self.m.positions.len() as u32;
            for &j in t {
                let p = pts[j as usize];
                self.m.positions.push((p - self.origin).to_array());
                self.m.normals.push(normal.to_array());
                self.m.colors.push(paint.rgba());
                self.m.uvs.push(uv.of(p));
                self.m.uvs_b.push(paint.uv_b());
            }
            let [a, b, c] = t.map(|j| pts[j as usize]);
            if ((b - a).cross(c - a).y > 0.0) == up {
                self.m.indices.extend([base, base + 1, base + 2]);
            } else {
                self.m.indices.extend([base, base + 2, base + 1]);
            }
        }
    }
}

/// Offsets a ring (outer counter-clockwise or hole clockwise in (x, z))
/// by `d` into the building (negative: out of it), mitred, the mitre
/// limited to 3 × `d`.
pub fn inset_ring(ring: &[[f32; 2]], d: f32) -> Vec<[f32; 2]> {
    let n = ring.len();
    if n < 3 || d == 0.0 {
        return ring.to_vec();
    }
    let outward = |i: usize| {
        let (a, b) = (Vec2::from(ring[i]), Vec2::from(ring[(i + 1) % n]));
        Vec2::new(b.y - a.y, a.x - b.x).normalize_or_zero()
    };
    (0..n)
        .map(|i| {
            let (n1, n2) = (outward((i + n - 1) % n), outward(i));
            let s = 1.0 + n1.dot(n2);
            let m = if s > 1e-3 {
                (n1 + n2) / s
            } else {
                n2 // a reversal: no mitre
            };
            let m = m.clamp_length_max(3.0);
            (Vec2::from(ring[i]) - m * d).to_array()
        })
        .collect()
}

/// Window bays on a wall of `len` metres: (count, u scale).
pub fn bays(len: f32, bay: f32) -> (u32, f32) {
    if bay <= 0.0 || len < 0.6 * bay {
        return (0, 1.0);
    }
    let n = (len / bay).round().max(1.0);
    (n as u32, n * bay / len)
}

/// The wall that gets the door: the longest outer wall of at least 3 m.
pub fn door_wall(outline: &[[f32; 2]]) -> Option<usize> {
    let n = outline.len();
    (0..n)
        .map(|i| {
            let (a, b) = (Vec2::from(outline[i]), Vec2::from(outline[(i + 1) % n]));
            (i, a.distance(b))
        })
        .filter(|(_, l)| *l >= 3.0)
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(i, _)| i)
}

/// Draws one building at a level of detail into `m` (positions relative
/// to `origin`). `windows` are the shader's window styles (for the bays).
pub fn house_arrays(
    b: &Building,
    h: &HouseSpec,
    lod: Lod,
    windows: &[WindowStyle],
    m: &mut MeshArrays,
    origin: Vec3,
) {
    let mut out = Out { m, origin };
    let inset = lod.inset();
    let near = lod == Lod::Near;
    let drop = if lod == Lod::Near { 0.0 } else { inset * 0.7 };
    let outline = inset_ring(&b.outline, inset);
    let holes: Vec<Vec<[f32; 2]>> = b.holes.iter().map(|r| inset_ring(r, inset)).collect();
    let eave = h.eave_y - if h.shape.is_pitched() { 0.0 } else { drop };
    // Façade of the walls.
    let window = h
        .window
        .filter(|_| lod != Lod::Far)
        .filter(|w| usize::from(*w) < MAX_WINDOW_STYLES)
        .and_then(|w| windows.get(usize::from(w)).map(|s| (w, s)));
    let storey_dm = (h.storey_h * 10.0).round() as u32;
    let wall_paint = |door: u32, has_windows: bool| Paint {
        lower: h.wall.layer,
        upper: h.upper.layer,
        colour: h.wall.colour,
        window: if has_windows {
            window.map_or(0, |(w, _)| u32::from(w) + 1)
        } else {
            0
        },
        storey_dm,
        levels: u32::from(h.levels),
        door,
    };
    let door_at = if h.canopy { None } else { door_wall(&outline) };
    let seed = crate::house_style::seed_of(b);
    // Walls (a canopy: the slab's edge; parapets above flat roofs, near).
    let parapet = if near && !h.shape.is_pitched() {
        h.parapet
    } else {
        0.0
    };
    let wall_top = eave + parapet;
    for (ri, ring) in std::iter::once(&outline).chain(&holes).enumerate() {
        let n = ring.len();
        for i in 0..n {
            let (a, c) = (Vec2::from(ring[i]), Vec2::from(ring[(i + 1) % n]));
            let len = a.distance(c);
            if len < 1e-3 {
                continue;
            }
            let dir = (c - a) / len;
            let outward = Vec3::new(dir.y, 0.0, -dir.x);
            let (count, scale) = window.map_or((0, 1.0), |(_, s)| bays(len, s.bay_m));
            // One door on the ground floor of the longest wall (a window
            // style without windows, like the garage's, still has its door).
            let door = match door_at {
                Some(d) if ri == 0 && d == i && count > 0 => {
                    let r = crate::house_style::chance(seed, 20);
                    ((r * count as f32) as u32).min(count - 1) + 1
                }
                _ => 0,
            };
            let paint = wall_paint(door, count > 0 && !h.canopy);
            let quad = [
                Vec3::new(a.x, h.wall_bottom, a.y),
                Vec3::new(c.x, h.wall_bottom, c.y),
                Vec3::new(c.x, wall_top, c.y),
                Vec3::new(a.x, wall_top, a.y),
            ];
            let uv = UvFrame {
                at: Vec3::new(a.x, h.ground_y, a.y),
                u: Vec3::new(dir.x, 0.0, dir.y) * scale,
                v: Vec3::Y,
                v0: 0.0,
            };
            out.face(&quad, outward, &paint, &uv);
        }
    }
    let roof_paint = Paint::plain(h.roof.layer, h.roof.colour);
    let gable_paint = Paint::plain(h.upper.layer, h.upper.colour);
    match h.shape {
        RoofShape::Flat => {
            let pts: Vec<Vec3> = outline
                .iter()
                .chain(holes.iter().flatten())
                .map(|p| Vec3::new(p[0], eave, p[1]))
                .collect();
            let tris = triangulate(&outline, &holes);
            if h.canopy {
                // The slab: top, bottom (and posts, near).
                out.tris(&pts, &tris, true, &roof_paint);
                let under: Vec<Vec3> = pts.iter().map(|p| p.with_y(h.wall_bottom)).collect();
                out.tris(&under, &tris, false, &gable_paint);
                if near {
                    for p in outline.iter().take(12) {
                        let c = Vec2::from(*p)
                            - (Vec2::from(*p) - centroid(&outline)).normalize_or_zero() * 0.3;
                        post(&mut out, c, b.base_y, h.wall_bottom, 0.12, &gable_paint);
                    }
                }
            } else {
                out.tris(&pts, &tris, true, &roof_paint);
                if parapet > 0.0 {
                    parapets(&mut out, &outline, &holes, eave, parapet, &gable_paint);
                }
            }
        }
        RoofShape::Pyramidal
            if outline.len() != 4
                || !matches!(b.roof, ocw_world::buildings::Roof::Gabled { .. }) =>
        {
            let o = if near { h.overhang } else { 0.0 };
            pyramid_on_polygon(&mut out, &outline, eave, h.roof_h, o, &roof_paint, o > 0.0);
        }
        shape => {
            if outline.len() != 4 {
                return;
            }
            let frame = RectFrame::new(&outline, eave);
            let o = if near { h.overhang } else { 0.0 };
            rect_roof(
                &mut out,
                &frame,
                shape,
                h.roof_h,
                o,
                &roof_paint,
                &gable_paint,
                near,
            );
            if near && h.chimney && h.roof_h > 1.0 {
                chimney(&mut out, &frame, h, seed, &gable_paint);
            }
        }
    }
}

fn centroid(ring: &[[f32; 2]]) -> Vec2 {
    ring.iter().map(|p| Vec2::from(*p)).sum::<Vec2>() / ring.len().max(1) as f32
}

/// A square post (box without a bottom) from `y0` to `y1`.
fn post(out: &mut Out, c: Vec2, y0: f32, y1: f32, r: f32, paint: &Paint) {
    let corners = [
        c + Vec2::new(-r, -r),
        c + Vec2::new(r, -r),
        c + Vec2::new(r, r),
        c + Vec2::new(-r, r),
    ];
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let mid = (a + b) / 2.0 - c;
        let q = [
            Vec3::new(a.x, y0, a.y),
            Vec3::new(b.x, y0, b.y),
            Vec3::new(b.x, y1, b.y),
            Vec3::new(a.x, y1, a.y),
        ];
        let uv = UvFrame::plane(q[0], q[1] - q[0], Vec3::new(mid.x, 0.0, mid.y));
        out.face(&q, Vec3::new(mid.x, 0.0, mid.y), paint, &uv);
    }
}

/// Parapets around a flat roof: the walls go on above it (drawn with the
/// walls), here the inner face and the cap.
fn parapets(
    out: &mut Out,
    outline: &[[f32; 2]],
    holes: &[Vec<[f32; 2]>],
    eave: f32,
    p: f32,
    paint: &Paint,
) {
    const T: f32 = 0.25;
    for ring in std::iter::once(&outline.to_vec()).chain(holes) {
        let inner = inset_ring(ring, T);
        let n = ring.len();
        for i in 0..n {
            let j = (i + 1) % n;
            let (a, c) = (Vec2::from(inner[i]), Vec2::from(inner[j]));
            let dir = (c - a).normalize_or_zero();
            // Inner face looks into the building (against the outward normal).
            let inward = -Vec3::new(dir.y, 0.0, -dir.x);
            let q = [
                Vec3::new(a.x, eave, a.y),
                Vec3::new(c.x, eave, c.y),
                Vec3::new(c.x, eave + p, c.y),
                Vec3::new(a.x, eave + p, a.y),
            ];
            let uv = UvFrame::plane(q[0], q[1] - q[0], inward);
            out.face(&q, inward, paint, &uv);
            let (oa, oc) = (Vec2::from(ring[i]), Vec2::from(ring[j]));
            let cap = [
                Vec3::new(oa.x, eave + p, oa.y),
                Vec3::new(oc.x, eave + p, oc.y),
                Vec3::new(c.x, eave + p, c.y),
                Vec3::new(a.x, eave + p, a.y),
            ];
            out.face(&cap, Vec3::Y, paint, &UvFrame::flat());
        }
    }
}

/// A pyramid over a (convex) polygon, apex above the centroid.
fn pyramid_on_polygon(
    out: &mut Out,
    ring: &[[f32; 2]],
    eave: f32,
    height: f32,
    overhang: f32,
    paint: &Paint,
    slab: bool,
) {
    let c = centroid(ring);
    let radius =
        ring.iter().map(|p| Vec2::from(*p).distance(c)).sum::<f32>() / ring.len().max(1) as f32;
    let slope = height / radius.max(0.5);
    let base = if overhang > 0.0 {
        inset_ring(ring, -overhang)
    } else {
        ring.to_vec()
    };
    let y0 = eave - overhang * slope;
    let apex = Vec3::new(c.x, eave + height, c.y);
    let n = base.len();
    for i in 0..n {
        let (a, b) = (Vec2::from(base[i]), Vec2::from(base[(i + 1) % n]));
        let tri = [Vec3::new(a.x, y0, a.y), Vec3::new(b.x, y0, b.y), apex];
        let mid = (a + b) / 2.0 - c;
        let outward = Vec3::new(mid.x, 0.0, mid.y).normalize_or_zero() + Vec3::Y;
        let uv = UvFrame::plane(tri[0], tri[1] - tri[0], outward);
        out.face(&tri, outward, paint, &uv);
        if slab {
            let under: Vec<Vec3> = tri.iter().map(|p| *p - Vec3::Y * ROOF_SLAB_M).collect();
            out.face(&under, -outward, paint, &uv);
            let edge = [tri[0], tri[1], under[1], under[0]];
            out.face(&edge, Vec3::new(mid.x, 0.0, mid.y), paint, &uv);
        }
    }
}

/// The local frame of a rectangular footprint: ridge along `ex` (the first
/// edge), across along `ez`, from corner 0 at the eaves.
#[derive(Debug, Clone, Copy)]
pub struct RectFrame {
    pub c0: Vec2,
    pub ex: Vec2,
    pub ez: Vec2,
    pub len: f32,
    pub width: f32,
    pub eave: f32,
}

impl RectFrame {
    pub fn new(c: &[[f32; 2]], eave: f32) -> Self {
        let (c0, c1, c3) = (Vec2::from(c[0]), Vec2::from(c[1]), Vec2::from(c[3]));
        Self {
            c0,
            ex: (c1 - c0).normalize_or_zero(),
            ez: (c3 - c0).normalize_or_zero(),
            len: c0.distance(c1),
            width: c0.distance(c3),
            eave,
        }
    }

    /// A point at (u along, v across, h above the eaves).
    pub fn at(&self, u: f32, v: f32, h: f32) -> Vec3 {
        let p = self.c0 + self.ex * u + self.ez * v;
        Vec3::new(p.x, self.eave + h, p.y)
    }

    fn dir(&self, u: f32, v: f32) -> Vec3 {
        let d = self.ex * u + self.ez * v;
        Vec3::new(d.x, 0.0, d.y)
    }
}

/// The roof planes and gables of a rectangular footprint as polygons in
/// local (u, v, h) coordinates of a frame `len` × `width`, eaves at h = 0.
/// Returns (roof planes, gable walls) with their outward directions.
#[allow(clippy::type_complexity)]
pub fn roof_polygons(
    shape: RoofShape,
    len: f32,
    width: f32,
    height: f32,
) -> (
    Vec<(Vec<[f32; 3]>, [f32; 2])>,
    Vec<(Vec<[f32; 3]>, [f32; 2])>,
) {
    let (l, w, hh) = (len, width, height);
    let half = w / 2.0;
    // Outward directions in (u, v): roof planes also face up.
    let (front, back, left, right) = ([0.0, -1.0], [0.0, 1.0], [-1.0, 0.0], [1.0, 0.0]);
    let mut roofs: Vec<(Vec<[f32; 3]>, [f32; 2])> = Vec::new();
    let mut gables: Vec<(Vec<[f32; 3]>, [f32; 2])> = Vec::new();
    match shape {
        RoofShape::Flat => {}
        RoofShape::Gabled => {
            roofs.push((
                vec![
                    [0.0, 0.0, 0.0],
                    [l, 0.0, 0.0],
                    [l, half, hh],
                    [0.0, half, hh],
                ],
                front,
            ));
            roofs.push((
                vec![[l, w, 0.0], [0.0, w, 0.0], [0.0, half, hh], [l, half, hh]],
                back,
            ));
            gables.push((vec![[l, 0.0, 0.0], [l, w, 0.0], [l, half, hh]], right));
            gables.push((vec![[0.0, w, 0.0], [0.0, 0.0, 0.0], [0.0, half, hh]], left));
        }
        RoofShape::Hipped | RoofShape::Pyramidal => {
            let k = if shape == RoofShape::Pyramidal {
                l / 2.0
            } else {
                half.min(l / 2.0)
            };
            let (r0, r1) = ([k, half, hh], [l - k, half, hh]);
            roofs.push((vec![[0.0, 0.0, 0.0], [l, 0.0, 0.0], r1, r0], front));
            roofs.push((vec![[l, w, 0.0], [0.0, w, 0.0], r0, r1], back));
            roofs.push((vec![[l, 0.0, 0.0], [l, w, 0.0], r1], right));
            roofs.push((vec![[0.0, w, 0.0], [0.0, 0.0, 0.0], r0], left));
        }
        RoofShape::HalfHipped => {
            // The gables reach 55 % of the height, the hips above them keep
            // the roof's pitch.
            let f = 0.55;
            let k = ((1.0 - f) * half).min(l / 2.0);
            let (e0, e1) = (f * half, w - f * half);
            let (r0, r1) = ([k, half, hh], [l - k, half, hh]);
            roofs.push((
                vec![
                    [0.0, 0.0, 0.0],
                    [l, 0.0, 0.0],
                    [l, e0, f * hh],
                    r1,
                    r0,
                    [0.0, e0, f * hh],
                ],
                front,
            ));
            roofs.push((
                vec![
                    [l, w, 0.0],
                    [0.0, w, 0.0],
                    [0.0, e1, f * hh],
                    r0,
                    r1,
                    [l, e1, f * hh],
                ],
                back,
            ));
            roofs.push((vec![[l, e0, f * hh], [l, e1, f * hh], r1], right));
            roofs.push((vec![[0.0, e1, f * hh], [0.0, e0, f * hh], r0], left));
            gables.push((
                vec![[l, 0.0, 0.0], [l, w, 0.0], [l, e1, f * hh], [l, e0, f * hh]],
                right,
            ));
            gables.push((
                vec![
                    [0.0, w, 0.0],
                    [0.0, 0.0, 0.0],
                    [0.0, e0, f * hh],
                    [0.0, e1, f * hh],
                ],
                left,
            ));
        }
        RoofShape::Skillion => {
            roofs.push((
                vec![[0.0, 0.0, 0.0], [l, 0.0, 0.0], [l, w, hh], [0.0, w, hh]],
                front,
            ));
            gables.push((
                vec![[l, w, 0.0], [0.0, w, 0.0], [0.0, w, hh], [l, w, hh]],
                back,
            ));
            gables.push((vec![[l, 0.0, 0.0], [l, w, 0.0], [l, w, hh]], right));
            gables.push((vec![[0.0, w, 0.0], [0.0, 0.0, 0.0], [0.0, w, hh]], left));
        }
        RoofShape::Mansard => {
            // Steep lower slopes to 65 % of the height over 18 % of the
            // shorter side, then a hipped top.
            let d = 0.18 * w.min(l);
            let h1 = 0.65 * hh;
            let (il, iw) = (l - 2.0 * d, w - 2.0 * d);
            let k = (iw / 2.0).min(il / 2.0);
            let (r0, r1) = ([d + k, half, hh], [l - d - k, half, hh]);
            let (i0, i1, i2, i3) = (
                [d, d, h1],
                [l - d, d, h1],
                [l - d, w - d, h1],
                [d, w - d, h1],
            );
            roofs.push((vec![[0.0, 0.0, 0.0], [l, 0.0, 0.0], i1, i0], front));
            roofs.push((vec![[l, 0.0, 0.0], [l, w, 0.0], i2, i1], right));
            roofs.push((vec![[l, w, 0.0], [0.0, w, 0.0], i3, i2], back));
            roofs.push((vec![[0.0, w, 0.0], [0.0, 0.0, 0.0], i0, i3], left));
            roofs.push((vec![i0, i1, r1, r0], front));
            roofs.push((vec![i2, i3, r0, r1], back));
            roofs.push((vec![i1, i2, r1], right));
            roofs.push((vec![i3, i0, r0], left));
        }
        RoofShape::Gambrel => {
            let d = 0.2 * w;
            let h1 = 0.6 * hh;
            let (b0, b1) = (d, w - d);
            roofs.push((
                vec![[0.0, 0.0, 0.0], [l, 0.0, 0.0], [l, b0, h1], [0.0, b0, h1]],
                front,
            ));
            roofs.push((
                vec![[0.0, b0, h1], [l, b0, h1], [l, half, hh], [0.0, half, hh]],
                front,
            ));
            roofs.push((
                vec![[l, w, 0.0], [0.0, w, 0.0], [0.0, b1, h1], [l, b1, h1]],
                back,
            ));
            roofs.push((
                vec![[l, b1, h1], [0.0, b1, h1], [0.0, half, hh], [l, half, hh]],
                back,
            ));
            gables.push((
                vec![
                    [l, 0.0, 0.0],
                    [l, w, 0.0],
                    [l, b1, h1],
                    [l, half, hh],
                    [l, b0, h1],
                ],
                right,
            ));
            gables.push((
                vec![
                    [0.0, w, 0.0],
                    [0.0, 0.0, 0.0],
                    [0.0, b0, h1],
                    [0.0, half, hh],
                    [0.0, b1, h1],
                ],
                left,
            ));
        }
    }
    (roofs, gables)
}

/// The roof over a rectangle with its gables; with an overhang `o` (near)
/// the roof planes are extended along their own pitch beyond the walls
/// (eaves by `o`, verges by [`VERGE_SHARE`] × `o`) and get a slab.
#[allow(clippy::too_many_arguments)]
fn rect_roof(
    out: &mut Out,
    f: &RectFrame,
    shape: RoofShape,
    height: f32,
    o: f32,
    roof: &Paint,
    gable: &Paint,
    slab: bool,
) {
    // Gables on the walls (no overhang).
    let (_, gables) = roof_polygons(shape, f.len, f.width, height);
    for (poly, dir) in &gables {
        let p: Vec<Vec3> = poly.iter().map(|q| f.at(q[0], q[1], q[2])).collect();
        let outward = f.dir(dir[0], dir[1]);
        let uv = UvFrame {
            at: p[0].with_y(0.0),
            u: (p[1] - p[0]).with_y(0.0).normalize_or_zero(),
            v: Vec3::Y,
            v0: 0.0,
        };
        out.face(&p, outward, gable, &uv);
    }
    // The roof, extended by the overhang along its pitch: the steepest
    // eave slope gives how far the eaves go down.
    let half = f.width / 2.0;
    let slope = match shape {
        RoofShape::Skillion => height / f.width.max(0.1),
        RoofShape::Mansard => 0.65 * height / (0.18 * f.width.min(f.len)).max(0.1),
        RoofShape::Gambrel => 0.6 * height / (0.2 * f.width).max(0.1),
        _ => height / half.max(0.1),
    };
    let gable_ended = matches!(
        shape,
        RoofShape::Gabled | RoofShape::Skillion | RoofShape::Gambrel | RoofShape::HalfHipped
    );
    let (ov, ou) = if gable_ended {
        (o, o * VERGE_SHARE)
    } else {
        (o, o)
    };
    let dy = ov * slope;
    let big = RectFrame {
        c0: f.c0 - f.ex * ou - f.ez * ov,
        len: f.len + 2.0 * ou,
        width: f.width + 2.0 * ov,
        eave: f.eave - dy,
        ..*f
    };
    let rise = match shape {
        RoofShape::Skillion => height + 2.0 * dy,
        _ => height + dy,
    };
    let (roofs, _) = roof_polygons(shape, big.len, big.width, rise);
    for (poly, dir) in &roofs {
        let p: Vec<Vec3> = poly.iter().map(|q| big.at(q[0], q[1], q[2])).collect();
        let outward = big.dir(dir[0], dir[1]) + Vec3::Y;
        let along = if dir[0] == 0.0 {
            f.dir(1.0, 0.0)
        } else {
            f.dir(0.0, 1.0)
        };
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        let uv = UvFrame::plane(p[0], along, if n.dot(outward) < 0.0 { -n } else { n });
        out.face(&p, outward, roof, &uv);
        if slab && o > 0.0 {
            let under: Vec<Vec3> = p.iter().map(|q| *q - Vec3::Y * ROOF_SLAB_M).collect();
            out.face(&under, -outward, gable, &uv);
            // The slab's edges (inner ones disappear inside the roof).
            for i in 0..p.len() {
                let j = (i + 1) % p.len();
                let e = [p[i], p[j], under[j], under[i]];
                let mid = (p[i] + p[j]) / 2.0;
                let centre = big.at(big.len / 2.0, big.width / 2.0, 0.0);
                let side = (mid - centre).with_y(0.0);
                out.face(&e, side, gable, &uv);
            }
        }
    }
}

/// A chimney on a pitched roof: a box through the roof near the ridge.
fn chimney(out: &mut Out, f: &RectFrame, h: &HouseSpec, seed: u64, paint: &Paint) {
    let r = crate::house_style::chance(seed, 30);
    let u = f.len * (0.25 + 0.5 * r);
    let v = f.width / 2.0 + 0.6_f32.min(f.width / 4.0);
    let c = f.at(u, v, 0.0);
    let s = 0.35;
    let (y0, y1) = (f.eave + h.roof_h * 0.4, f.eave + h.roof_h + 0.9);
    let ex = Vec3::new(f.ex.x, 0.0, f.ex.y) * s;
    let ez = Vec3::new(f.ez.x, 0.0, f.ez.y) * s;
    let corners = [c - ex - ez, c + ex - ez, c + ex + ez, c - ex + ez];
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let q = [a.with_y(y0), b.with_y(y0), b.with_y(y1), a.with_y(y1)];
        let outward = ((a + b) / 2.0 - c).with_y(0.0);
        let uv = UvFrame::plane(q[0], b - a, outward);
        out.face(&q, outward, paint, &uv);
    }
    let top: Vec<Vec3> = corners.iter().map(|p| p.with_y(y1)).collect();
    out.face(&top, Vec3::Y, paint, &UvFrame::flat());
}

/// Footprint area of an outline (m²).
pub fn outline_area(outline: &[[f32; 2]]) -> f32 {
    (ocw_world::buildings::signed_area(outline) / 2.0).abs() as f32
}

/// The visibility ranges of the near, mid and far meshes (from their
/// origins): near fades out over [`NEAR_END_M`], mid over [`MID_END_M`],
/// far appears at [`FAR_START_M`]. Coverage: wherever a tile is closer
/// than [`FAR_START_M`] its mid mesh is drawn; the near chunks only add
/// detail on top.
pub fn lod_ranges() -> [bevy::camera::visibility::VisibilityRange; 3] {
    use bevy::camera::visibility::VisibilityRange;
    [
        VisibilityRange {
            start_margin: 0.0..0.0,
            end_margin: NEAR_END_M[0]..NEAR_END_M[1],
            use_aabb: false,
        },
        VisibilityRange {
            start_margin: 0.0..0.0,
            end_margin: MID_END_M[0]..MID_END_M[1],
            use_aabb: false,
        },
        VisibilityRange::abrupt(FAR_START_M, f32::MAX),
    ]
}

/// One house mesh with the point it is placed at (the distance for its
/// level of detail is measured from there).
#[derive(Default)]
pub struct HousePart {
    pub origin: [f32; 3],
    pub mesh: MeshArrays,
}

/// The houses of a tile in their levels of detail.
#[derive(Default)]
pub struct HouseMeshes {
    /// Per [`NEAR_CHUNK_M`] chunk with buildings.
    pub near: Vec<HousePart>,
    pub mid: Option<HousePart>,
    pub far: Option<HousePart>,
}

impl HouseMeshes {
    /// Triangles of the near (all chunks), mid and far meshes.
    pub fn triangles(&self) -> [usize; 3] {
        [
            self.near.iter().map(|p| p.mesh.indices.len() / 3).sum(),
            self.mid.as_ref().map_or(0, |p| p.mesh.indices.len() / 3),
            self.far.as_ref().map_or(0, |p| p.mesh.indices.len() / 3),
        ]
    }

    pub fn is_empty(&self) -> bool {
        self.triangles() == [0, 0, 0]
    }
}

/// The look of each building of a package tile: the regional style chosen
/// by the package's time zone, the building's place and elevation
/// ([`crate::house_style::BuildingStyles::select`]), resolved with its
/// measured roof colour (`roofs[i]`).
pub fn house_specs(
    pkg: &ocw_world::package::WorldPackage,
    buildings: &[Building],
    roofs: &[Option<ocw_world::look::RoofColour>],
    look: &crate::house_style::HouseLook,
) -> Vec<HouseSpec> {
    let styles = &*crate::house_style::STYLES;
    let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let tz = pkg.time_zone.as_deref();
    buildings
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let c = centroid(&b.outline);
            let ll = proj.unproject([f64::from(c.x), f64::from(c.y)]);
            let elevation = f64::from(b.base_y + crate::house_style::SINK_M);
            let style = styles.select(tz, ll, elevation);
            crate::house_style::resolve(b, roofs.get(i).copied().flatten(), styles, style, look)
        })
        .collect()
}

/// The meshes of a tile's houses: near chunks, the merged mid and far
/// meshes (see the module docs). `tile_origin` is the tile's corner,
/// `size` its side (m).
pub fn tile_houses(
    buildings: &[Building],
    specs: &[HouseSpec],
    tile_origin: [f32; 2],
    size: f32,
) -> HouseMeshes {
    let windows = &crate::house_style::STYLES.windows;
    if buildings.is_empty() {
        return HouseMeshes::default();
    }
    let n = (size / NEAR_CHUNK_M).ceil().max(1.0) as usize;
    let chunk_of = |b: &Building| {
        let c = centroid(&b.outline);
        let i = (((c.x - tile_origin[0]) / NEAR_CHUNK_M).floor().max(0.0) as usize).min(n - 1);
        let k = (((c.y - tile_origin[1]) / NEAR_CHUNK_M).floor().max(0.0) as usize).min(n - 1);
        k * n + i
    };
    let mean_y = |list: &mut dyn Iterator<Item = &Building>| {
        let (s, c) = list.fold((0.0, 0), |(s, c), b| (s + b.base_y, c + 1));
        s / c.max(1) as f32
    };
    let mut chunks: Vec<Vec<usize>> = vec![Vec::new(); n * n];
    for (j, b) in buildings.iter().enumerate() {
        chunks[chunk_of(b)].push(j);
    }
    let mut out = HouseMeshes::default();
    for (ci, list) in chunks.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        let (i, k) = (ci % n, ci / n);
        let y = mean_y(&mut list.iter().map(|&j| &buildings[j]));
        let origin = Vec3::new(
            tile_origin[0] + (i as f32 + 0.5) * NEAR_CHUNK_M,
            y,
            tile_origin[1] + (k as f32 + 0.5) * NEAR_CHUNK_M,
        );
        let mut mesh = MeshArrays::default();
        for &j in list {
            house_arrays(
                &buildings[j],
                &specs[j],
                Lod::Near,
                windows,
                &mut mesh,
                origin,
            );
        }
        out.near.push(HousePart {
            origin: origin.to_array(),
            mesh,
        });
    }
    let centre = Vec3::new(
        tile_origin[0] + size / 2.0,
        mean_y(&mut buildings.iter()),
        tile_origin[1] + size / 2.0,
    );
    let mut mid = MeshArrays::default();
    let mut far = MeshArrays::default();
    for (b, h) in buildings.iter().zip(specs) {
        house_arrays(b, h, Lod::Mid, windows, &mut mid, centre);
        if outline_area(&b.outline) >= FAR_MIN_AREA_M2 {
            house_arrays(b, h, Lod::Far, windows, &mut far, centre);
        }
    }
    let part = |mesh: MeshArrays| {
        (!mesh.indices.is_empty()).then(|| HousePart {
            origin: centre.to_array(),
            mesh,
        })
    };
    out.mid = part(mid);
    out.far = part(far);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::house_style::{HouseSpec, Surface};
    use ocw_world::buildings::{BuildingKind, BuildingTags, Roof};
    use std::collections::HashMap;

    fn spec(shape: RoofShape, roof_h: f32) -> HouseSpec {
        let s = Surface {
            layer: Layer::Plaster,
            colour: [0.8; 3],
        };
        HouseSpec {
            shape,
            roof_h,
            eave_y: 106.0,
            wall_bottom: 99.5,
            ground_y: 100.0,
            overhang: 0.6,
            parapet: 0.5,
            wall: s,
            upper: s,
            roof: Surface {
                layer: Layer::RoofTiles,
                colour: [0.5, 0.2, 0.1],
            },
            window: Some(0),
            levels: 2,
            storey_h: 3.0,
            chimney: false,
            canopy: false,
        }
    }

    fn building(roof: Roof) -> Building {
        Building {
            kind: BuildingKind::House,
            outline: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]],
            holes: vec![],
            base_y: 99.5,
            eave_y: 106.0,
            roof,
            tags: BuildingTags::default(),
        }
    }

    fn windows() -> Vec<WindowStyle> {
        crate::house_style::STYLES.windows.clone()
    }

    fn draw(b: &Building, h: &HouseSpec, lod: Lod) -> MeshArrays {
        let mut m = MeshArrays::default();
        house_arrays(b, h, lod, &windows(), &mut m, Vec3::ZERO);
        m
    }

    /// Edges by welded position: every edge above the open bottom must be
    /// shared by exactly two triangles in opposite directions.
    fn assert_closed(m: &MeshArrays, bottom: f32) {
        let key = |p: [f32; 3]| {
            (
                (p[0] * 1000.0).round() as i64,
                (p[1] * 1000.0).round() as i64,
                (p[2] * 1000.0).round() as i64,
            )
        };
        type Point = (i64, i64, i64);
        let mut edges: HashMap<(Point, Point), i32> = HashMap::new();
        for t in m.indices.chunks(3) {
            for k in 0..3 {
                let (a, b) = (
                    key(m.positions[t[k] as usize]),
                    key(m.positions[t[(k + 1) % 3] as usize]),
                );
                if a == b {
                    continue;
                }
                *edges.entry((a, b)).or_default() += 1;
            }
        }
        let bottom = (bottom * 1000.0).round() as i64;
        for (&(a, b), &n) in &edges {
            if a.1 == bottom && b.1 == bottom {
                continue; // the open foot of the walls
            }
            let back = edges.get(&(b, a)).copied().unwrap_or(0);
            assert_eq!((n, back), (1, 1), "edge {a:?} → {b:?}");
        }
    }

    fn y_range(m: &MeshArrays) -> (f32, f32) {
        m.positions
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p[1]), hi.max(p[1]))
            })
    }

    /// Every roof shape over a rectangle is closed with its walls (mid
    /// LOD: no overhang), reaches its height and faces outwards.
    #[test]
    fn roof_shapes_are_watertight_with_their_heights() {
        for shape in RoofShape::ALL.iter().copied() {
            let roof = if shape == RoofShape::Flat {
                Roof::Flat
            } else {
                Roof::Gabled { height_m: 3.0 }
            };
            let h = HouseSpec {
                parapet: 0.0,
                ..spec(shape, if shape == RoofShape::Flat { 0.0 } else { 3.0 })
            };
            // Mid without the inset: exactly the footprint.
            let mut m = MeshArrays::default();
            let b = building(roof);
            house_arrays(&b, &h, Lod::Near, &windows(), &mut m, Vec3::ZERO);
            let o = HouseSpec { overhang: 0.0, ..h };
            let mut closed = MeshArrays::default();
            house_arrays(&b, &o, Lod::Near, &windows(), &mut closed, Vec3::ZERO);
            assert_closed(&closed, 99.5);
            let (lo, hi) = y_range(&closed);
            assert_eq!(lo, 99.5, "{shape:?}");
            assert!(
                (hi - (106.0 + h.roof_h)).abs() < 1e-4,
                "{shape:?}: top {hi}"
            );
            // Every triangle winds towards its normal, and the normal
            // points away from the house's centre (the shapes are convex).
            let centre = Vec3::new(6.0, 103.0, 4.0);
            for tri in closed.indices.chunks(3) {
                let [a, b, c] = [0, 1, 2].map(|j| Vec3::from(closed.positions[tri[j] as usize]));
                let n = Vec3::from(closed.normals[tri[0] as usize]);
                assert!(
                    (b - a).cross(c - a).dot(n) > 0.0,
                    "{shape:?}: winding at {a}"
                );
                assert!(
                    n.dot((a + b + c) / 3.0 - centre) > 0.0,
                    "{shape:?}: inward at {a}"
                );
            }
            assert!(!m.indices.is_empty());
        }
    }

    #[test]
    fn ridges_follow_the_first_edge_and_skillions_rise_away_from_it() {
        // Ridge points of a gable: all top vertices at v = 4 (the middle of
        // the 8 m side), spanning u = 0 … 12.
        let b = building(Roof::Gabled { height_m: 3.0 });
        let h = HouseSpec {
            overhang: 0.0,
            ..spec(RoofShape::Gabled, 3.0)
        };
        let m = draw(&b, &h, Lod::Near);
        let top: Vec<[f32; 3]> = m
            .positions
            .iter()
            .copied()
            .filter(|p| (p[1] - 109.0).abs() < 1e-4)
            .collect();
        assert!(top.iter().all(|p| (p[2] - 4.0).abs() < 1e-4));
        assert!(
            top.iter().any(|p| p[0].abs() < 1e-4) && top.iter().any(|p| (p[0] - 12.0).abs() < 1e-4)
        );
        // A hipped ridge is shorter by the half width at both ends.
        let m = draw(
            &b,
            &HouseSpec {
                overhang: 0.0,
                ..spec(RoofShape::Hipped, 3.0)
            },
            Lod::Near,
        );
        let xs: Vec<f32> = m
            .positions
            .iter()
            .filter(|p| (p[1] - 109.0).abs() < 1e-4)
            .map(|p| p[0])
            .collect();
        let (lo, hi) = xs
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
        assert!(
            (lo - 4.0).abs() < 1e-4 && (hi - 8.0).abs() < 1e-4,
            "{lo} {hi}"
        );
        // A skillion: high along the far side (v = 8), low along the first edge.
        let m = draw(
            &b,
            &HouseSpec {
                overhang: 0.0,
                ..spec(RoofShape::Skillion, 2.0)
            },
            Lod::Near,
        );
        for p in m.positions.iter().filter(|p| (p[1] - 108.0).abs() < 1e-4) {
            assert!((p[2] - 8.0).abs() < 1e-4, "{p:?}");
        }
        // A pyramid: one apex above the centre.
        let m = draw(
            &b,
            &HouseSpec {
                overhang: 0.0,
                ..spec(RoofShape::Pyramidal, 4.0)
            },
            Lod::Near,
        );
        for p in m.positions.iter().filter(|p| (p[1] - 110.0).abs() < 1e-4) {
            assert!(
                (p[0] - 6.0).abs() < 1e-4 && (p[2] - 4.0).abs() < 1e-4,
                "{p:?}"
            );
        }
    }

    #[test]
    fn overhangs_extend_the_roof_along_its_pitch() {
        let b = building(Roof::Gabled { height_m: 3.0 });
        let h = spec(RoofShape::Gabled, 3.0); // overhang 0.6
        let m = draw(&b, &h, Lod::Near);
        // Roof faces (tile layer) reach 0.6 m beyond the long walls and 0.42 m
        // beyond the gables, lowered along the pitch (3 m over 4 m).
        let roof: Vec<[f32; 3]> = m
            .positions
            .iter()
            .zip(&m.uvs_b)
            .filter(|(_, b)| b[0] == 0.0)
            .map(|(p, _)| *p)
            .collect();
        let zmin = roof.iter().map(|p| p[2]).fold(f32::MAX, f32::min);
        let xmax = roof.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
        let ymin = roof.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!((zmin + 0.6).abs() < 1e-4, "{zmin}");
        assert!((xmax - 12.42).abs() < 1e-4, "{xmax}");
        assert!((ymin - (106.0 - 0.6 * 0.75)).abs() < 1e-4, "{ymin}");
        // Mid: no overhang, walls and roof inside the near ones.
        let mid = draw(&b, &h, Lod::Mid);
        let (lo_x, hi_x) = mid
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, c), p| (a.min(p[0]), c.max(p[0])));
        assert!((lo_x - MID_INSET_M).abs() < 1e-4 && (hi_x - (12.0 - MID_INSET_M)).abs() < 1e-4);
        assert!(mid.indices.len() < m.indices.len());
    }

    #[test]
    fn walls_carry_windows_storeys_and_one_door() {
        let b = building(Roof::Gabled { height_m: 3.0 });
        let h = spec(RoofShape::Gabled, 3.0);
        let m = draw(&b, &h, Lod::Mid);
        let ws = &windows()[0];
        let mut doors = std::collections::BTreeSet::new();
        for (uvb, c) in m.uvs_b.iter().zip(&m.colors) {
            let (lo, up, w, storey, levels) = decode_uv_b(*uvb);
            if w > 0 {
                assert_eq!((lo, up), (Layer::Plaster as u32, Layer::Plaster as u32));
                assert_eq!(w, 1);
                assert!((storey - 3.0).abs() < 1e-4);
                assert_eq!(levels, 2);
            }
            if c[3] > 0.0 {
                doors.insert(c[3] as u32);
            }
        }
        assert_eq!(doors.len(), 1, "one door: {doors:?}");
        // The long wall (11.7 m) gets round(11.7 / 3.4) = 3 bays; the door is one of them.
        assert!(*doors.iter().next().unwrap() <= 3);
        let (n, s) = bays(11.7, ws.bay_m);
        assert_eq!(n, 3);
        assert!((n as f32 * ws.bay_m / 11.7 - s).abs() < 1e-6);
        assert_eq!(bays(1.0, 3.4), (0, 1.0));
        // Far: no windows, no doors.
        let far = draw(&b, &h, Lod::Far);
        assert!(far.uvs_b.iter().all(|u| decode_uv_b(*u).2 == 0));
        assert!(far.colors.iter().all(|c| c[3] == 0.0));
        // Encoding round trip.
        let p = Paint {
            lower: Layer::Plaster,
            upper: Layer::Timber,
            colour: [0.5; 3],
            window: 3,
            storey_dm: 28,
            levels: 4,
            door: 2,
        };
        assert_eq!(decode_uv_b(p.uv_b()), (1, 2, 3, 2.8, 4));
    }

    #[test]
    fn flat_roofs_parapets_courtyards_and_canopies() {
        let mut b = building(Roof::Flat);
        b.outline = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        b.holes = vec![vec![[8.0, 8.0], [8.0, 12.0], [12.0, 12.0], [12.0, 8.0]]];
        let h = spec(RoofShape::Flat, 0.0);
        let m = draw(&b, &h, Lod::Near);
        let (_, hi) = y_range(&m);
        assert!((hi - 106.5).abs() < 1e-4, "parapet top {hi}");
        // Without the parapet the roof closes the walls, courtyard included.
        let m = draw(&b, &HouseSpec { parapet: 0.0, ..h }, Lod::Near);
        assert_closed(&m, 99.5);
        // Mid: no parapet, roof a little lower (inside the near one).
        let mid = draw(&b, &h, Lod::Mid);
        let (_, hi) = y_range(&mid);
        assert!(hi < 106.0 && hi > 105.8, "{hi}");
        // A canopy: a slab with posts, no walls down to the ground.
        let c = HouseSpec {
            canopy: true,
            wall_bottom: 105.65,
            window: None,
            parapet: 0.0,
            ..h
        };
        let m = draw(&building(Roof::Flat), &c, Lod::Near);
        let low = m.positions.iter().filter(|p| p[1] < 100.0).count();
        assert_eq!(low, 4 * 4 * 2, "four posts reach the ground");
        let mid = draw(&building(Roof::Flat), &c, Lod::Mid);
        assert!(mid.positions.iter().all(|p| p[1] > 105.0));
    }

    #[test]
    fn insets_shrink_rings_and_courtyards() {
        let sq = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert_eq!(
            inset_ring(&sq, 1.0),
            vec![[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0]]
        );
        // A clockwise hole grows (into the building).
        let hole = [[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]];
        assert_eq!(
            inset_ring(&hole, 1.0),
            vec![[3.0, 3.0], [3.0, 7.0], [7.0, 7.0], [7.0, 3.0]]
        );
        assert_eq!(inset_ring(&sq, -0.5)[0], [-0.5, -0.5]);
        assert_eq!(
            door_wall(&[[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]]),
            Some(0)
        );
        assert_eq!(door_wall(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]]), None);
    }

    #[test]
    fn pyramids_on_polygons_and_chimneys() {
        let ring: Vec<[f32; 2]> = (0..8)
            .map(|i| {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                [3.0 * a.cos(), 3.0 * a.sin()]
            })
            .collect();
        let tower = Building {
            outline: ring.clone(),
            roof: Roof::Flat,
            ..building(Roof::Flat)
        };
        let h = HouseSpec {
            overhang: 0.0,
            ..spec(RoofShape::Pyramidal, 10.0)
        };
        let m = draw(&tower, &h, Lod::Mid);
        let (_, hi) = y_range(&m);
        assert!((hi - 116.0).abs() < 1e-4);
        let near = draw(&tower, &HouseSpec { overhang: 0.0, ..h }, Lod::Near);
        assert_closed(&near, 99.5);
        // A chimney rises 0.9 m above the ridge.
        let b = building(Roof::Gabled { height_m: 3.0 });
        let h = HouseSpec {
            chimney: true,
            ..spec(RoofShape::Gabled, 3.0)
        };
        let (_, hi) = y_range(&draw(&b, &h, Lod::Near));
        assert!((hi - 109.9).abs() < 1e-4, "{hi}");
        let (_, hi) = y_range(&draw(&b, &h, Lod::Mid));
        assert!(hi < 109.0 + 1e-3, "no chimney in mid: {hi}");
    }
}
