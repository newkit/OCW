//! House meshes (spec §14 Phase 8 T4): walls with procedural façades,
//! roof shapes (flat, gabled, hipped, half-hipped, pyramidal, skillion,
//! mansard, gambrel), eaves and parapets, in three levels of detail.
//!
//! Each building is resolved to a [`HouseSpec`] (`house_style`) and drawn
//! with the building material (`building.wesl`): every face carries its
//! material layer(s) and façade code in the second texture coordinate,
//! (along, up) in metres in the first, its colour (linear) in the vertex
//! colour and the door's bay in the colour's alpha (on walls with windows
//! plus 64 × the building's light seed, [`Paint::light_seed`]: which
//! windows are lit at night, deterministic per building):
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
use ocw_world::roof_skeleton::{Skeleton, skeleton, triangulate_face};

use crate::house_style::{BalconySide, Dormer, HouseSpec, Layer, MAX_WINDOW_STYLES, WindowStyle};
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
    /// The building's light seed (0 … [`LIGHT_SEEDS`] − 1; walls with
    /// windows): its lit windows at night (`building.wesl`,
    /// `building_material::window_light`).
    pub light_seed: u32,
}

/// The number of light seeds (packed into the colour's alpha above the
/// door's bay: exact in an f32).
pub const LIGHT_SEEDS: u32 = 4096;

/// Decodes the colour's alpha (the shader's twin): (door bay + 1, light
/// seed).
pub fn decode_alpha(a: f32) -> (u32, u32) {
    let a = a.round() as u32;
    (a % 64, a / 64)
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
            light_seed: 0,
        }
    }

    pub fn uv_b(&self) -> [f32; 2] {
        let code = |l: Layer| f32::from(l as u8);
        // Storeys always (the upper floors switch material by them), the
        // window style only where windows are drawn.
        let facade = self.window + 16 * (self.storey_dm.min(63) + 64 * self.levels.min(16383));
        [code(self.lower) + 16.0 * code(self.upper), facade as f32]
    }

    pub fn rgba(&self) -> [f32; 4] {
        [
            self.colour[0],
            self.colour[1],
            self.colour[2],
            if self.window > 0 {
                (self.door.min(63) + 64 * (self.light_seed % LIGHT_SEEDS)) as f32
            } else {
                self.door as f32
            },
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

    /// Triangles by index into `pts` (a roof face over a polygon), all with
    /// one normal, wound towards it.
    fn poly(
        &mut self,
        pts: &[Vec3],
        tris: &[[usize; 3]],
        normal: Vec3,
        paint: &Paint,
        uv: &UvFrame,
    ) {
        let base = self.m.positions.len() as u32;
        for p in pts {
            self.m.positions.push((*p - self.origin).to_array());
            self.m.normals.push(normal.to_array());
            self.m.colors.push(paint.rgba());
            self.m.uvs.push(uv.of(*p));
            self.m.uvs_b.push(paint.uv_b());
        }
        for t in tris {
            let [a, b, c] = t.map(|i| pts[i]);
            let [i, j, k] = t.map(|i| base + i as u32);
            if (b - a).cross(c - a).dot(normal) >= 0.0 {
                self.m.indices.extend([i, j, k]);
            } else {
                self.m.indices.extend([i, k, j]);
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
    house_arrays_inset(b, h, lod, 0.0, windows, m, origin);
}

/// [`house_arrays`] with the walls and roof set `extra` further inside
/// (the parts drawn on top of an outline they do not cover).
#[allow(clippy::too_many_arguments)]
fn house_arrays_inset(
    b: &Building,
    h: &HouseSpec,
    lod: Lod,
    extra: f32,
    windows: &[WindowStyle],
    m: &mut MeshArrays,
    origin: Vec3,
) {
    let mut out = Out { m, origin };
    let inset = lod.inset() + extra;
    let near = lod == Lod::Near;
    let drop = if lod == Lod::Near { 0.0 } else { inset * 0.7 };
    // The outline the near level sees (its roof overhangs it).
    let base: Vec<[f32; 2]> = inset_ring(&b.outline, extra);
    let base_holes: Vec<Vec<[f32; 2]>> = b.holes.iter().map(|r| inset_ring(r, extra)).collect();
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
    let seed = h.seed;
    let light_seed = (seed ^ (seed >> 29) ^ (seed >> 47)) as u32 % LIGHT_SEEDS;
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
        light_seed,
    };
    let door_at = if h.canopy { None } else { door_wall(&outline) };
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
    // The roof faces' detail in the mid level (dormers, roof windows).
    let detail = (lod == Lod::Mid).then_some(light_seed);
    let dormer_paints = DormerPaints {
        front: Paint {
            lower: h.upper.layer,
            upper: h.upper.layer,
            colour: h.upper.colour,
            ..wall_paint(0, true)
        },
        window: window.map(|(_, s)| s.clone()),
        cheek: gable_paint,
        roof: roof_paint,
    };
    let mut gable_ends: Vec<usize> = Vec::new();
    match h.shape {
        RoofShape::Flat => {
            if h.canopy {
                // The slab: top, bottom (and posts, near).
                let pts: Vec<Vec3> = outline
                    .iter()
                    .chain(holes.iter().flatten())
                    .map(|p| Vec3::new(p[0], eave, p[1]))
                    .collect();
                let tris = triangulate(&outline, &holes);
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
                flat_roof(&mut out, &outline, &holes, eave, &roof_paint);
                if parapet > 0.0 {
                    parapets(&mut out, &outline, &holes, eave, parapet, &gable_paint);
                }
            }
        }
        RoofShape::Pyramidal
            if !h.skeleton
                && (outline.len() != 4
                    || !matches!(b.roof, ocw_world::buildings::Roof::Gabled { .. })) =>
        {
            let o = if near { h.overhang } else { 0.0 };
            pyramid_on_polygon(&mut out, &outline, eave, h.roof_h, o, &roof_paint, o > 0.0);
        }
        _ if h.skeleton => {
            let gabled = h.shape == RoofShape::Gabled;
            let Some((full, top)) = roof_skeleton(&base, &base_holes, gabled) else {
                // No skeleton for this footprint: a flat roof.
                flat_roof(&mut out, &outline, &holes, eave, &roof_paint);
                return;
            };
            let slope = h.roof_h / top.max(0.1);
            let rows = h
                .dormers
                .map(|k| skeleton_rows(&full, eave, slope, k))
                .unwrap_or_default();
            gable_ends = (0..b.outline.len()).filter(|&e| full.gable[e]).collect();
            let o = if near { h.overhang } else { 0.0 };
            let shell = (o > 0.0)
                .then(|| {
                    let ring = inset_ring(&base, -o);
                    let hs: Vec<Vec<[f32; 2]>> =
                        base_holes.iter().map(|r| inset_ring(r, -o)).collect();
                    roof_skeleton(&ring, &hs, gabled)
                })
                .flatten();
            let inner = if inset > extra {
                roof_skeleton(&outline, &holes, gabled).map(|(s, _)| s)
            } else {
                Some(full.clone())
            };
            match (shell, inner) {
                (Some((shell, _)), _) if near => {
                    // The gables on the walls, the roof overhanging them.
                    skeleton_roof(
                        &mut out,
                        &full,
                        eave,
                        slope,
                        &SkeletonPaint {
                            roof: &roof_paint,
                            gable: &gable_paint,
                            gables: true,
                            planes: false,
                            slab: false,
                        },
                        &[],
                        None,
                    );
                    skeleton_roof(
                        &mut out,
                        &shell,
                        eave - slope * o,
                        slope,
                        &SkeletonPaint {
                            roof: &roof_paint,
                            gable: &gable_paint,
                            gables: false,
                            planes: true,
                            slab: true,
                        },
                        &[],
                        None,
                    );
                }
                (_, Some(inner)) => skeleton_roof(
                    &mut out,
                    &inner,
                    eave,
                    slope,
                    &SkeletonPaint {
                        roof: &roof_paint,
                        gable: &gable_paint,
                        gables: true,
                        planes: true,
                        slab: false,
                    },
                    &rows,
                    detail,
                ),
                _ => flat_roof(&mut out, &outline, &holes, eave, &roof_paint),
            }
            if near {
                for (_, row) in &rows {
                    for k in 0..row.count {
                        dormer(&mut out, row, k, &dormer_paints);
                    }
                }
                if h.chimney && h.roof_h > 1.0 {
                    skeleton_chimney(&mut out, &full, eave, slope, h, &gable_paint);
                }
            }
        }
        shape => {
            if outline.len() != 4 {
                return;
            }
            let frame = RectFrame::new(&outline, eave);
            let rows = h
                .dormers
                .map(|k| rect_rows(&base, h, k))
                .unwrap_or_default();
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
                &rows,
                detail,
            );
            if matches!(
                shape,
                RoofShape::Gabled | RoofShape::HalfHipped | RoofShape::Gambrel
            ) {
                gable_ends = vec![1, 3];
            }
            if near {
                for (_, row) in &rows {
                    for k in 0..row.count {
                        dormer(&mut out, row, k, &dormer_paints);
                    }
                }
                if h.chimney && h.roof_h > 1.0 {
                    chimney(&mut out, &frame, h, seed, &gable_paint);
                }
            }
        }
    }
    if near && let Some(side) = h.balcony {
        balconies(&mut out, &base, h, side, &gable_ends);
    }
}

/// A flat roof over the outline and its courtyards.
fn flat_roof(
    out: &mut Out,
    outline: &[[f32; 2]],
    holes: &[Vec<[f32; 2]>],
    eave: f32,
    paint: &Paint,
) {
    let pts: Vec<Vec3> = outline
        .iter()
        .chain(holes.iter().flatten())
        .map(|p| Vec3::new(p[0], eave, p[1]))
        .collect();
    let tris = triangulate(outline, holes);
    out.tris(&pts, &tris, true, paint);
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
    rows: &[([f32; 2], RoofRow)],
    detail: Option<u32>,
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
        let row = rows.iter().find(|(d, _)| d == dir).map(|(_, r)| r);
        let (paint, uv) = match (row, detail) {
            (Some(r), Some(seed)) => (r.paint(roof, seed), r.uv()),
            _ => (
                *roof,
                UvFrame::plane(p[0], along, if n.dot(outward) < 0.0 { -n } else { n }),
            ),
        };
        out.face(&p, outward, &paint, &uv);
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
    let (y0, y1) = (f.eave + h.roof_h * 0.4, f.eave + h.roof_h + 0.9);
    chimney_box(out, Vec2::new(c.x, c.z), f.ex, f.ez, y0, y1, paint);
}

/// A chimney box (0.7 m square, sides along `ex` / `ez`) from `y0` to `y1`.
fn chimney_box(out: &mut Out, c: Vec2, ex: Vec2, ez: Vec2, y0: f32, y1: f32, paint: &Paint) {
    let s = 0.35;
    let c = Vec3::new(c.x, 0.0, c.y);
    let ex = Vec3::new(ex.x, 0.0, ex.y) * s;
    let ez = Vec3::new(ez.x, 0.0, ez.y) * s;
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

// --- Roofs over polygons (straight skeleton, v3.26) -------------------------

/// The straight skeleton of an outline (`ocw_world::roof_skeleton`), with
/// gables where `gabled` and the ends allow, and its largest offset before
/// the gables (the roof height is slope × that).
pub fn roof_skeleton(
    outline: &[[f32; 2]],
    holes: &[Vec<[f32; 2]>],
    gabled: bool,
) -> Option<(Skeleton, f32)> {
    let ring = |r: &[[f32; 2]]| -> Vec<[f64; 2]> {
        r.iter()
            .map(|p| [f64::from(p[0]), f64::from(p[1])])
            .collect()
    };
    let hs: Vec<Vec<[f64; 2]>> = holes.iter().map(|h| ring(h)).collect();
    let mut sk = skeleton(&ring(outline), &hs)?;
    let top = sk.max_offset as f32;
    if top < 0.3 {
        return None;
    }
    if gabled {
        sk.make_gables();
    }
    Some((sk, top))
}

/// What [`skeleton_roof`] draws, and in what.
struct SkeletonPaint<'a> {
    roof: &'a Paint,
    gable: &'a Paint,
    /// The vertical gables (on the walls).
    gables: bool,
    /// The sloped faces.
    planes: bool,
    /// A slab under the sloped faces with its edges (the near overhang).
    slab: bool,
}

/// The faces of a skeleton roof: sloped faces at `eave + slope × offset`,
/// vertical gables; `rows` give their faces the shader's detail (`detail`:
/// the light seed, mid level).
fn skeleton_roof(
    out: &mut Out,
    sk: &Skeleton,
    eave: f32,
    slope: f32,
    p: &SkeletonPaint,
    rows: &[(usize, RoofRow)],
    detail: Option<u32>,
) {
    let pos = |i: usize| {
        let n = sk.nodes[i];
        Vec3::new(n.p[0] as f32, eave + slope * n.t as f32, n.p[1] as f32)
    };
    let shared: std::collections::HashSet<(usize, usize)> = sk
        .faces
        .iter()
        .enumerate()
        .filter(|(e, _)| !sk.gable[*e])
        .flat_map(|(_, f)| (0..f.len()).map(move |k| (f[k], f[(k + 1) % f.len()])))
        .collect();
    for (e, f) in sk.faces.iter().enumerate() {
        let (a, c) = (sk.nodes[f[0]].p, sk.nodes[f[1]].p);
        let d = Vec2::new((c[0] - a[0]) as f32, (c[1] - a[1]) as f32).normalize_or_zero();
        let n_in = Vec2::new(-d.y, d.x);
        let pts: Vec<Vec3> = f.iter().map(|&i| pos(i)).collect();
        if sk.gable[e] {
            if p.gables {
                let uv = UvFrame {
                    at: pts[0].with_y(0.0),
                    u: Vec3::new(d.x, 0.0, d.y),
                    v: Vec3::Y,
                    v0: 0.0,
                };
                out.face(&pts, Vec3::new(-n_in.x, 0.0, -n_in.y), p.gable, &uv);
            }
            continue;
        }
        if !p.planes {
            continue;
        }
        let normal = Vec3::new(-slope * n_in.x, 1.0, -slope * n_in.y).normalize();
        let plan: Vec<[f64; 2]> = f.iter().map(|&i| sk.nodes[i].p).collect();
        let tris = triangulate_face(&plan);
        let row = rows.iter().find(|(re, _)| *re == e).map(|(_, r)| r);
        let (paint, uv) = match (row, detail) {
            (Some(r), Some(seed)) => (r.paint(p.roof, seed), r.uv()),
            _ => (
                *p.roof,
                UvFrame::plane(pts[0], Vec3::new(d.x, 0.0, d.y), normal),
            ),
        };
        out.poly(&pts, &tris, normal, &paint, &uv);
        if p.slab {
            let under: Vec<Vec3> = pts.iter().map(|q| *q - Vec3::Y * ROOF_SLAB_M).collect();
            out.poly(&under, &tris, -normal, p.gable, &uv);
            let centre = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
            for k in 0..f.len() {
                let (i, j) = (f[k], f[(k + 1) % f.len()]);
                if shared.contains(&(j, i)) {
                    continue; // inside the roof
                }
                let (pa, pb) = (pts[k], pts[(k + 1) % f.len()]);
                let edge = [
                    pa,
                    pb,
                    pb - Vec3::Y * ROOF_SLAB_M,
                    pa - Vec3::Y * ROOF_SLAB_M,
                ];
                let side = ((pa + pb) / 2.0 - centre).with_y(0.0);
                out.face(&edge, side, p.gable, &uv);
            }
        }
    }
}

/// A chimney on a skeleton roof: near the highest node, on the side of the
/// longest eave.
fn skeleton_chimney(
    out: &mut Out,
    sk: &Skeleton,
    eave: f32,
    slope: f32,
    h: &HouseSpec,
    paint: &Paint,
) {
    let Some(e) = (0..sk.faces.len())
        .filter(|e| !sk.gable[*e])
        .max_by(|x, y| {
            let len = |e: usize| {
                let f = &sk.faces[e];
                let (a, b) = (sk.nodes[f[0]].p, sk.nodes[f[1]].p);
                (b[0] - a[0]).hypot(b[1] - a[1])
            };
            len(*x).total_cmp(&len(*y)).then(y.cmp(x))
        })
    else {
        return;
    };
    let f = &sk.faces[e];
    let Some(&top) = f
        .iter()
        .max_by(|x, y| sk.nodes[**x].t.total_cmp(&sk.nodes[**y].t))
    else {
        return;
    };
    let (a, b) = (sk.nodes[f[0]].p, sk.nodes[f[1]].p);
    let d = Vec2::new((b[0] - a[0]) as f32, (b[1] - a[1]) as f32).normalize_or_zero();
    let n_in = Vec2::new(-d.y, d.x);
    let n = sk.nodes[top];
    let back = (0.6_f32).min(n.t as f32 / 2.0);
    let c = Vec2::new(n.p[0] as f32, n.p[1] as f32) - n_in * back;
    let y_top = eave + slope * n.t as f32 + 0.9;
    chimney_box(out, c, d, n_in, eave + h.roof_h * 0.4, y_top, paint);
}

// --- Dormers and roof windows (v3.26) ---------------------------------------

/// The front wall of a dormer above the roof (m).
pub const DORMER_FRONT_M: f32 = 1.45;
/// Dormers reach this far into the roof below its surface (m).
pub const DORMER_SINK_M: f32 = 0.12;
/// A shed dormer's slope as a share of the roof's.
pub const SHED_SLOPE: f32 = 0.25;
/// A roof window: width and length up the slope (m).
pub const ROOF_WINDOW_M: [f32; 2] = [0.8, 1.2];
/// The window code of the roof faces' detail in the shader (dormers and
/// roof windows painted in the mid level; one past the window styles).
pub const ROOF_DETAIL: u32 = 15;

/// The width of a dormer (or roof window) of a kind (m).
pub fn dormer_width(k: Dormer) -> f32 {
    match k {
        Dormer::Gabled => 1.8,
        Dormer::Shed => 2.6,
        Dormer::Window => ROOF_WINDOW_M[0],
    }
}

/// The kind's code in the roof detail (the shader's twin).
pub fn dormer_code(k: Dormer) -> u32 {
    match k {
        Dormer::Window => 1,
        Dormer::Gabled => 2,
        Dormer::Shed => 3,
    }
}

/// Dormers or roof windows in a row on one roof plane, planned on the
/// outline without insets so that every level puts them in one place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoofRow {
    pub kind: Dormer,
    /// The point of the eave line (at the eaves' height) where the first
    /// cell starts.
    pub at: Vec3,
    /// Along the eave and into the house (horizontal unit vectors).
    pub along: Vec3,
    pub inward: Vec3,
    /// Rise per horizontal metre.
    pub slope: f32,
    /// The fronts (a roof window's foot) this far from the eave line.
    pub front: f32,
    /// The same up the slope, in dm (exact: the shader's code).
    pub up_dm: u32,
    pub count: u32,
    /// Cell size along the eave in dm; the first centre half a cell in.
    pub spacing_dm: u32,
}

impl RoofRow {
    pub fn spacing(&self) -> f32 {
        self.spacing_dm as f32 / 10.0
    }

    /// The centre of cell `k` on the eave line.
    pub fn centre(&self, k: u32) -> Vec3 {
        self.at + self.along * ((k as f32 + 0.5) * self.spacing())
    }

    /// The uv frame of its roof face: u along the eave from `at`, v up the
    /// slope from the eave line.
    fn uv(&self) -> UvFrame {
        UvFrame {
            at: self.at,
            u: self.along,
            v: (self.inward + Vec3::Y * self.slope).normalize(),
            v0: 0.0,
        }
    }

    /// Its roof face's paint for the shader's detail: window code
    /// [`ROOF_DETAIL`], the storey field the foot up the slope (dm), the
    /// levels field kind + 4 × count + 64 × spacing (dm).
    fn paint(&self, roof: &Paint, light_seed: u32) -> Paint {
        Paint {
            window: ROOF_DETAIL,
            storey_dm: self.up_dm,
            levels: self.code(),
            light_seed,
            ..*roof
        }
    }

    pub fn code(&self) -> u32 {
        dormer_code(self.kind) + 4 * self.count.min(15) + 64 * self.spacing_dm.min(127)
    }
}

/// The parts of the line at depth `d` inside a polygon in (u, d).
fn section(poly: &[[f32; 2]], d: f32) -> Vec<(f32, f32)> {
    let n = poly.len();
    let mut xs: Vec<f32> = (0..n)
        .filter_map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            ((a[1] > d) != (b[1] > d)).then(|| a[0] + (d - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
        })
        .collect();
    xs.sort_by(f32::total_cmp);
    xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect()
}

fn longest(v: Vec<(f32, f32)>) -> Option<(f32, f32)> {
    v.into_iter()
        .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
}

/// Plans a row of dormers (or roof windows) on a roof plane given as a
/// polygon in (u along its eave, d horizontally into the house) rising at
/// `slope`: (first cell's start u, front d, front up the slope in dm,
/// count, cell size in dm); `None` where none fits under the ridge.
pub fn plan_row(poly: &[[f32; 2]], slope: f32, kind: Dormer) -> Option<(f32, f32, u32, u32, u32)> {
    if slope < 0.35 || poly.len() < 3 {
        return None;
    }
    let depth = poly.iter().map(|p| p[1]).fold(0.0, f32::max);
    if depth < 2.5 {
        return None;
    }
    let k = (1.0 + slope * slope).sqrt();
    let up = ((0.3 * depth).clamp(0.9, 3.5) * k * 10.0).round().min(63.0);
    let front = up / 10.0 / k;
    let w = dormer_width(kind);
    let back = match kind {
        Dormer::Gabled => {
            let top = slope * front + DORMER_FRONT_M + w / 2.0 * slope.max(0.6);
            (top + DORMER_SINK_M) / slope
        }
        Dormer::Shed => front + (DORMER_FRONT_M + DORMER_SINK_M) / (slope * (1.0 - SHED_SLOPE)),
        Dormer::Window => front + ROOF_WINDOW_M[1] / k,
    };
    if back > depth - 0.3 {
        return None;
    }
    let (a0, a1) = longest(section(poly, front))?;
    let (b0, b1) = longest(section(poly, back))?;
    let (u0, u1) = (a0.max(b0) + 0.4, a1.min(b1) - 0.4);
    let usable = u1 - u0;
    if usable < w + 0.2 {
        return None;
    }
    let pitch = match kind {
        Dormer::Gabled => (w + 1.6).max(3.2),
        Dormer::Shed => w + 1.8,
        Dormer::Window => 2.2,
    };
    let cap = if kind == Dormer::Window { 6 } else { 4 };
    let n = ((usable / pitch).floor() as u32).clamp(1, cap);
    let cell = if n == 1 { usable.min(pitch) } else { pitch };
    let spacing_dm = ((cell * 10.0).floor() as u32).min(127);
    let start = u0 + (usable - n as f32 * spacing_dm as f32 / 10.0) / 2.0;
    Some((start, front, up as u32, n, spacing_dm))
}

/// The rows on a rectangular roof (gabled, hipped, half-hipped): one on
/// each long side where they fit; tagged with the plane's direction in
/// the frame (front `[0, −1]`, back `[0, 1]`).
pub fn rect_rows(outline: &[[f32; 2]], h: &HouseSpec, kind: Dormer) -> Vec<([f32; 2], RoofRow)> {
    if outline.len() != 4
        || !matches!(
            h.shape,
            RoofShape::Gabled | RoofShape::Hipped | RoofShape::HalfHipped
        )
    {
        return Vec::new();
    }
    let f = RectFrame::new(outline, h.eave_y);
    let slope = h.roof_h / (f.width / 2.0).max(0.1);
    let (roofs, _) = roof_polygons(h.shape, f.len, f.width, h.roof_h);
    let mut out = Vec::new();
    for (poly, dir) in roofs {
        let back = match dir {
            [0.0, -1.0] => false,
            [0.0, 1.0] => true,
            _ => continue,
        };
        let ud: Vec<[f32; 2]> = poly
            .iter()
            .map(|q| {
                if back {
                    [f.len - q[0], f.width - q[1]]
                } else {
                    [q[0], q[1]]
                }
            })
            .collect();
        let Some((start, front, up_dm, count, spacing_dm)) = plan_row(&ud, slope, kind) else {
            continue;
        };
        let (at, along, inward) = if back {
            (
                f.at(f.len - start, f.width, 0.0),
                -f.dir(1.0, 0.0),
                -f.dir(0.0, 1.0),
            )
        } else {
            (f.at(start, 0.0, 0.0), f.dir(1.0, 0.0), f.dir(0.0, 1.0))
        };
        out.push((
            dir,
            RoofRow {
                kind,
                at,
                along,
                inward,
                slope,
                front,
                up_dm,
                count,
                spacing_dm,
            },
        ));
    }
    out
}

/// The rows on a skeleton roof: one per sloped face over an eave of at
/// least 5 m, by the face's edge.
pub fn skeleton_rows(sk: &Skeleton, eave: f32, slope: f32, kind: Dormer) -> Vec<(usize, RoofRow)> {
    let mut out = Vec::new();
    for (e, f) in sk.faces.iter().enumerate() {
        if sk.gable[e] {
            continue;
        }
        let (a, b) = (sk.nodes[f[0]].p, sk.nodes[f[1]].p);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]) as f32;
        if len < 5.0 {
            continue;
        }
        let d = Vec2::new((b[0] - a[0]) as f32, (b[1] - a[1]) as f32) / len;
        let n_in = Vec2::new(-d.y, d.x);
        let a2 = Vec2::new(a[0] as f32, a[1] as f32);
        let ud: Vec<[f32; 2]> = f
            .iter()
            .map(|&i| {
                let n = sk.nodes[i];
                let q = Vec2::new(n.p[0] as f32, n.p[1] as f32) - a2;
                [q.dot(d), n.t as f32]
            })
            .collect();
        let Some((start, front, up_dm, count, spacing_dm)) = plan_row(&ud, slope, kind) else {
            continue;
        };
        let at = a2 + d * start;
        out.push((
            e,
            RoofRow {
                kind,
                at: Vec3::new(at.x, eave, at.y),
                along: Vec3::new(d.x, 0.0, d.y),
                inward: Vec3::new(n_in.x, 0.0, n_in.y),
                slope,
                front,
                up_dm,
                count,
                spacing_dm,
            },
        ));
    }
    out
}

/// How dormers are painted.
struct DormerPaints {
    /// The front wall with the house's window style (one storey).
    front: Paint,
    window: Option<WindowStyle>,
    cheek: Paint,
    roof: Paint,
}

/// Dormer `k` of a row (near level): a closed solid standing on the roof
/// (its bottom just under the roof surface, inside the house); a roof
/// window: a glazed panel a little above the slope.
fn dormer(out: &mut Out, row: &RoofRow, k: u32, p: &DormerPaints) {
    let c = row.centre(k);
    let s = row.slope;
    let w = dormer_width(row.kind) / 2.0;
    let at = |u: f32, d: f32, y: f32| c + row.along * u + row.inward * d + Vec3::Y * y;
    let df = row.front;
    let pf = s * df;
    let normal = (Vec3::Y - row.inward * s).normalize();
    let out_d = -row.inward;
    let up_slope = (row.inward + Vec3::Y * s).normalize();
    if row.kind == Dormer::Window {
        let len = ROOF_WINDOW_M[1];
        let base = [
            at(-w, df, pf),
            at(w, df, pf),
            at(w, df, pf) + up_slope * len,
            at(-w, df, pf) + up_slope * len,
        ];
        let top: Vec<Vec3> = base.iter().map(|q| *q + normal * 0.08).collect();
        let low: Vec<Vec3> = base.iter().map(|q| *q - normal * 0.03).collect();
        let glass = Paint::plain(Layer::Glass, [0.02, 0.025, 0.03]);
        let frame = Paint::plain(Layer::Metal, [0.05, 0.05, 0.055]);
        out.face(
            &top,
            normal,
            &glass,
            &UvFrame::plane(top[0], row.along, normal),
        );
        let centre = (base[0] + base[2]) / 2.0;
        for i in 0..4 {
            let j = (i + 1) % 4;
            let q = [low[i], low[j], top[j], top[i]];
            let side = ((base[i] + base[j]) / 2.0 - centre).normalize_or_zero();
            out.face(&q, side, &frame, &UvFrame::plane(q[0], q[1] - q[0], side));
        }
        return;
    }
    let y1 = pf + DORMER_FRONT_M;
    let sink = DORMER_SINK_M;
    let (fbl, fbr) = (at(-w, df, pf - sink), at(w, df, pf - sink));
    let (ftl, ftr) = (at(-w, df, y1), at(w, df, y1));
    // The front: one window from the house's style, centred, its top a
    // little under the dormer's eave.
    let front_uv = |front: &[Vec3]| -> (Paint, UvFrame) {
        match &p.window {
            Some(ws) => {
                let storey = ws.sill_m + ws.height_m + 0.3;
                let paint = Paint {
                    storey_dm: (storey * 10.0).ceil().min(63.0) as u32,
                    levels: 1,
                    door: 0,
                    ..p.front
                };
                let y_at = c.y + y1 - 0.25 - (ws.sill_m + ws.height_m);
                (
                    paint,
                    UvFrame {
                        at: fbl.with_y(y_at),
                        u: row.along * (ws.bay_m / (2.0 * w)),
                        v: Vec3::Y,
                        v0: 0.0,
                    },
                )
            }
            None => (
                Paint {
                    window: 0,
                    ..p.front
                },
                UvFrame::plane(front[0], row.along, out_d),
            ),
        }
    };
    match row.kind {
        Dormer::Gabled => {
            let r = w * s.max(0.6);
            let yr = y1 + r;
            let rf = at(0.0, df, yr);
            let dv = (y1 + sink) / s;
            let drb = (yr + sink) / s;
            let (vl, vr) = (at(-w, dv, y1), at(w, dv, y1));
            let rb = at(0.0, drb, yr);
            let front = [fbl, fbr, ftr, rf, ftl];
            let (fp, fuv) = front_uv(&front);
            out.face(&front, out_d, &fp, &fuv);
            let side_uv = |q: Vec3, n: Vec3| UvFrame::plane(q, row.inward, n);
            out.face(
                &[fbl, ftl, vl],
                -row.along,
                &p.cheek,
                &side_uv(fbl, -row.along),
            );
            out.face(
                &[fbr, vr, ftr],
                row.along,
                &p.cheek,
                &side_uv(fbr, row.along),
            );
            let nl = -row.along + Vec3::Y;
            let nr = row.along + Vec3::Y;
            out.face(
                &[ftl, rf, rb, vl],
                nl,
                &p.roof,
                &UvFrame::plane(ftl, row.inward, nl),
            );
            out.face(
                &[ftr, vr, rb, rf],
                nr,
                &p.roof,
                &UvFrame::plane(ftr, row.inward, nr),
            );
            out.face(&[fbl, vl, rb, vr, fbr], -normal, &p.cheek, &UvFrame::flat());
        }
        Dormer::Shed => {
            let sd = SHED_SLOPE * s;
            let db = df + (DORMER_FRONT_M + sink) / (s - sd);
            let yb = y1 + sd * (db - df);
            let (bl, br) = (at(-w, db, yb), at(w, db, yb));
            let front = [fbl, fbr, ftr, ftl];
            let (fp, fuv) = front_uv(&front);
            out.face(&front, out_d, &fp, &fuv);
            let side_uv = |q: Vec3, n: Vec3| UvFrame::plane(q, row.inward, n);
            out.face(
                &[fbl, ftl, bl],
                -row.along,
                &p.cheek,
                &side_uv(fbl, -row.along),
            );
            out.face(
                &[fbr, br, ftr],
                row.along,
                &p.cheek,
                &side_uv(fbr, row.along),
            );
            let up = (Vec3::Y - row.inward * sd).normalize();
            out.face(
                &[ftl, ftr, br, bl],
                up,
                &p.roof,
                &UvFrame::plane(ftl, row.along, up),
            );
            out.face(&[fbl, bl, br, fbr], -normal, &p.cheek, &UvFrame::flat());
        }
        Dormer::Window => {}
    }
}

// --- Balconies (v3.26) ------------------------------------------------------

/// A balcony's depth out of the wall (m).
pub const BALCONY_DEPTH_M: f32 = 1.1;
/// Its railing's height above the floor (m).
pub const BALCONY_RAIL_M: f32 = 1.0;

/// A box over `base + u·[u0, u1] + n·[n0, n1]` from `y0` to `y1` (`u`, `n`
/// horizontal unit vectors, `n = u` turned to the right: (u.z, −u.x)).
#[allow(clippy::too_many_arguments)]
fn cuboid(
    out: &mut Out,
    base: Vec3,
    u: Vec3,
    n: Vec3,
    us: [f32; 2],
    ns: [f32; 2],
    ys: [f32; 2],
    paint: &Paint,
) {
    let p = |i: usize, j: usize, y: f32| (base + u * us[i] + n * ns[j]).with_y(y);
    let ring = [(0, 0), (1, 0), (1, 1), (0, 1)];
    for k in 0..4 {
        let ((i0, j0), (i1, j1)) = (ring[k], ring[(k + 1) % 4]);
        let q = [
            p(i0, j0, ys[0]),
            p(i1, j1, ys[0]),
            p(i1, j1, ys[1]),
            p(i0, j0, ys[1]),
        ];
        let mid = (q[0] + q[1]) / 2.0;
        let centre = (p(0, 0, 0.0) + p(1, 1, 0.0)) / 2.0;
        let side = (mid - centre).with_y(0.0);
        out.face(&q, side, paint, &UvFrame::plane(q[0], q[1] - q[0], side));
    }
    let top: Vec<Vec3> = ring.iter().map(|&(i, j)| p(i, j, ys[1])).collect();
    out.face(&top, Vec3::Y, paint, &UvFrame::flat());
    let bottom: Vec<Vec3> = ring.iter().map(|&(i, j)| p(i, j, ys[0])).collect();
    out.face(&bottom, Vec3::NEG_Y, paint, &UvFrame::flat());
}

/// The wall a balcony runs along: on a gable end (`gable_ends`, else the
/// short walls) or an eave side (the long walls); of those the one facing
/// south the most (z is south), then the longer.
pub fn balcony_wall(
    outline: &[[f32; 2]],
    side: BalconySide,
    gable_ends: &[usize],
) -> Option<usize> {
    let n = outline.len();
    let wall = |i: usize| {
        let (a, c) = (Vec2::from(outline[i]), Vec2::from(outline[(i + 1) % n]));
        let len = a.distance(c);
        let dir = (c - a) / len.max(1e-6);
        (len, Vec2::new(dir.y, -dir.x))
    };
    let longest = (0..n).map(|i| wall(i).0).fold(0.0, f32::max);
    let candidates: Vec<usize> = (0..n)
        .filter(|&i| {
            let (len, _) = wall(i);
            match side {
                BalconySide::Gable if !gable_ends.is_empty() => {
                    gable_ends.contains(&i) && len >= 4.0
                }
                BalconySide::Gable => len >= 4.0 && len < 0.8 * longest,
                BalconySide::Long => len >= 6.0 && len >= 0.8 * longest && !gable_ends.contains(&i),
            }
        })
        .collect();
    candidates.into_iter().max_by(|&i, &j| {
        let (li, ni) = wall(i);
        let (lj, nj) = wall(j);
        (ni.y + 0.01 * li)
            .total_cmp(&(nj.y + 0.01 * lj))
            .then(j.cmp(&i))
    })
}

/// Timber balconies on the upper storeys (near level): a slab and a
/// boarded railing along one wall, on each upper floor that has room
/// below the eaves (on a gable end also inside the gable, as wide as the
/// gable is there), at most two.
fn balconies(
    out: &mut Out,
    outline: &[[f32; 2]],
    h: &HouseSpec,
    side: BalconySide,
    gable_ends: &[usize],
) {
    let Some(i) = balcony_wall(outline, side, gable_ends) else {
        return;
    };
    let n = outline.len();
    let (a, c) = (Vec2::from(outline[i]), Vec2::from(outline[(i + 1) % n]));
    let len = a.distance(c);
    let dir = (c - a) / len;
    let u = Vec3::new(dir.x, 0.0, dir.y);
    let nrm = Vec3::new(dir.y, 0.0, -dir.x);
    let base = Vec3::new(a.x, 0.0, a.y);
    let timber = if h.upper.layer == Layer::Timber {
        h.upper.colour
    } else {
        Layer::Timber.fallback_colour()
    };
    let paint = Paint::plain(Layer::Timber, timber.map(|v| v * 0.8));
    let in_gable = gable_ends.contains(&i) && h.shape.is_pitched();
    let mut drawn = 0;
    for k in 1..u32::from(h.levels).max(1) + 1 {
        if drawn == 2 {
            break;
        }
        let y = h.ground_y + k as f32 * h.storey_h;
        let head = y + 2.3;
        // How wide the wall is at head height.
        let width = if head <= h.eave_y {
            len
        } else if in_gable && h.roof_h > 0.0 {
            len * (1.0 - (head - h.eave_y) / h.roof_h)
        } else {
            0.0
        };
        let margin = 0.3;
        let span = match side {
            BalconySide::Gable => width - 2.0 * margin,
            BalconySide::Long => (width - 2.0 * margin).min(14.0),
        };
        if span < 2.5 {
            continue;
        }
        let u0 = (len - span) / 2.0;
        let us = [u0, u0 + span];
        let d = BALCONY_DEPTH_M;
        cuboid(out, base, u, nrm, us, [-0.05, d], [y - 0.14, y], &paint);
        let t = 0.06;
        let top = y + BALCONY_RAIL_M;
        cuboid(out, base, u, nrm, us, [d - t, d], [y, top], &paint);
        cuboid(
            out,
            base,
            u,
            nrm,
            [us[0], us[0] + t],
            [0.0, d - t],
            [y, top],
            &paint,
        );
        cuboid(
            out,
            base,
            u,
            nrm,
            [us[1] - t, us[1]],
            [0.0, d - t],
            [y, top],
            &paint,
        );
        drawn += 1;
    }
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

/// The resolved looks of a tile's buildings and of their parts.
#[derive(Debug, Clone, Default)]
pub struct TileSpecs {
    pub specs: Vec<HouseSpec>,
    /// Per building the looks of its parts (`Building::parts`, in order).
    pub parts: Vec<Vec<HouseSpec>>,
}

impl TileSpecs {
    /// The top of building `i`: its own, or its highest part's.
    pub fn top(&self, i: usize) -> f32 {
        let own = self.specs.get(i).map_or(f32::MIN, HouseSpec::top);
        self.parts
            .get(i)
            .into_iter()
            .flatten()
            .map(HouseSpec::top)
            .fold(own, f32::max)
    }
}

/// The look of each building of a package tile: the regional style chosen
/// by the package's time zone, the building's place and elevation
/// ([`crate::house_style::BuildingStyles::select`]), resolved with its
/// measured roof colour (`roofs[i]`). Its parts take the building's style
/// and seed, and the measured roof colour when they cover at least 30 %
/// of the outline.
pub fn house_specs(
    pkg: &ocw_world::package::WorldPackage,
    buildings: &[Building],
    roofs: &[Option<ocw_world::look::RoofColour>],
    look: &crate::house_style::HouseLook,
) -> TileSpecs {
    let styles = &*crate::house_style::STYLES;
    let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let tz = pkg.time_zone.as_deref();
    let mut out = TileSpecs::default();
    for (i, b) in buildings.iter().enumerate() {
        let c = centroid(&b.outline);
        let ll = proj.unproject([f64::from(c.x), f64::from(c.y)]);
        let elevation = f64::from(b.base_y + crate::house_style::SINK_M);
        let style = styles.select(tz, ll, elevation);
        let measured = roofs.get(i).copied().flatten();
        let spec = crate::house_style::resolve(b, measured, styles, style, look);
        let area = outline_area(&b.outline);
        let parts = b
            .parts
            .list
            .iter()
            .map(|p| {
                let m = measured.filter(|_| outline_area(&p.outline) >= 0.3 * area);
                crate::house_style::resolve_seeded(p, spec.seed, m, styles, style, look)
            })
            .collect();
        out.specs.push(spec);
        out.parts.push(parts);
    }
    out
}

/// Parts drawn on top of an outline they do not cover sit this far inside
/// their own walls (no z-fighting where their walls meet the outline's).
pub const PART_INSET_M: f32 = 0.05;

/// The meshes of a tile's houses: near chunks, the merged mid and far
/// meshes (see the module docs). `tile_origin` is the tile's corner,
/// `size` its side (m). A building whose parts cover it is drawn as its
/// parts; one with other parts, as itself with the parts on top.
pub fn tile_houses(
    buildings: &[Building],
    specs: &TileSpecs,
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
    // Each building as what is drawn of it.
    let draw = |j: usize, lod: Lod, m: &mut MeshArrays, origin: Vec3| {
        let b = &buildings[j];
        let parts = specs.parts.get(j).map(Vec::as_slice).unwrap_or(&[]);
        let whole = b.parts.cover && parts.len() == b.parts.list.len() && !parts.is_empty();
        if !whole {
            house_arrays_inset(b, &specs.specs[j], lod, 0.0, windows, m, origin);
        }
        let extra = if whole { 0.0 } else { PART_INSET_M };
        for (p, h) in b.parts.list.iter().zip(parts) {
            house_arrays_inset(p, h, lod, extra, windows, m, origin);
        }
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
            draw(j, Lod::Near, &mut mesh, origin);
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
    for (j, b) in buildings.iter().enumerate() {
        draw(j, Lod::Mid, &mut mid, centre);
        if outline_area(&b.outline) >= FAR_MIN_AREA_M2 {
            draw(j, Lod::Far, &mut far, centre);
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
            skeleton: false,
            dormers: None,
            balcony: None,
            seed: 0x5EED_1234,
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
            parts: Default::default(),
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
            let (door, _) = decode_alpha(c[3]);
            if door > 0 {
                doors.insert(door);
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
            light_seed: 4095,
        };
        assert_eq!(decode_uv_b(p.uv_b()), (1, 2, 3, 2.8, 4));
        assert_eq!(decode_alpha(p.rgba()[3]), (2, 4095));
        // Walls without windows carry no seed (the far LOD: alpha 0).
        let plain = Paint { window: 0, ..p };
        assert_eq!(decode_alpha(plain.rgba()[3]), (2, 0));
        // Every wall of one building has the same seed, buildings differ.
        let seeds: std::collections::BTreeSet<u32> = m
            .uvs_b
            .iter()
            .zip(&m.colors)
            .filter(|(u, _)| decode_uv_b(**u).2 > 0)
            .map(|(_, c)| decode_alpha(c[3]).1)
            .collect();
        assert_eq!(seeds.len(), 1, "{seeds:?}");
        let mut other = b.clone();
        other.outline.iter_mut().for_each(|p| p[0] += 37.0);
        let m2 = draw(
            &other,
            &HouseSpec {
                seed: crate::house_style::seed_of(&other),
                ..h
            },
            Lod::Near,
        );
        let seed2 = m2
            .uvs_b
            .iter()
            .zip(&m2.colors)
            .find(|(u, _)| decode_uv_b(**u).2 > 0)
            .map(|(_, c)| decode_alpha(c[3]).1)
            .unwrap();
        assert_ne!(Some(&seed2), seeds.iter().next());
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

    // --- v3.26: roofs over polygons, dormers, balconies, parts --------------

    fn polygon(outline: Vec<[f32; 2]>, holes: Vec<Vec<[f32; 2]>>) -> Building {
        Building {
            outline,
            holes,
            roof: Roof::Flat,
            tags: BuildingTags {
                skeleton_roof_m: Some(3.2),
                ..Default::default()
            },
            ..building(Roof::Flat)
        }
    }

    fn l_house() -> Building {
        polygon(
            vec![
                [0.0, 0.0],
                [20.0, 0.0],
                [20.0, 8.0],
                [8.0, 8.0],
                [8.0, 20.0],
                [0.0, 20.0],
            ],
            vec![],
        )
    }

    fn skeleton_spec(shape: RoofShape, roof_h: f32) -> HouseSpec {
        HouseSpec {
            skeleton: true,
            overhang: 0.0,
            ..spec(shape, roof_h)
        }
    }

    /// The signed volume of a mesh open only at its foot (`bottom`): with
    /// the origin on the foot's plane the missing bottom adds nothing.
    fn volume(m: &MeshArrays, bottom: f32) -> f32 {
        m.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|j| {
                    let p = m.positions[t[j] as usize];
                    Vec3::new(p[0], p[1] - bottom, p[2])
                });
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    /// The highest point of the mesh above (x, z).
    fn top_at(m: &MeshArrays, x: f32, z: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for t in m.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|j| Vec3::from(m.positions[t[j] as usize]));
            let d = (b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z);
            if d.abs() < 1e-9 {
                continue;
            }
            let u = ((x - a.x) * (c.z - a.z) - (c.x - a.x) * (z - a.z)) / d;
            let v = ((b.x - a.x) * (z - a.z) - (x - a.x) * (b.z - a.z)) / d;
            if u < -1e-5 || v < -1e-5 || u + v > 1.0 + 1e-5 {
                continue;
            }
            let y = a.y + u * (b.y - a.y) + v * (c.y - a.y);
            best = Some(best.map_or(y, |q| q.max(y)));
        }
        best
    }

    /// Hipped and gabled roofs over L-, T- and U-shaped, odd and courtyard
    /// footprints are closed with their walls, face outwards (a positive
    /// volume of walls plus roof), reach their height with the ridges in
    /// the arms' middles, and a gabled one has its gables on the walls.
    #[test]
    fn skeleton_roofs_are_watertight_with_their_heights_and_ridges() {
        let t_shape = vec![
            [0.0, 0.0],
            [24.0, 0.0],
            [24.0, 8.0],
            [16.0, 8.0],
            [16.0, 18.0],
            [8.0, 18.0],
            [8.0, 8.0],
            [0.0, 8.0],
        ];
        let u_shape = vec![
            [0.0, 0.0],
            [18.0, 0.0],
            [18.0, 16.0],
            [12.0, 16.0],
            [12.0, 6.0],
            [6.0, 6.0],
            [6.0, 16.0],
            [0.0, 16.0],
        ];
        let odd = vec![
            [0.0, 0.0],
            [14.0, 1.0],
            [17.0, 7.0],
            [11.0, 9.0],
            [12.0, 15.0],
            [2.0, 13.0],
        ];
        let court = (
            vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
            vec![vec![[6.0, 6.0], [6.0, 14.0], [14.0, 14.0], [14.0, 6.0]]],
        );
        let shapes = [
            l_house(),
            polygon(t_shape, vec![]),
            polygon(u_shape, vec![]),
            polygon(odd, vec![]),
            polygon(court.0, court.1),
        ];
        for (i, b) in shapes.iter().enumerate() {
            let area =
                outline_area(&b.outline) - b.holes.iter().map(|h| outline_area(h)).sum::<f32>();
            for shape in [RoofShape::Hipped, RoofShape::Gabled] {
                let h = skeleton_spec(shape, 3.2);
                let m = draw(b, &h, Lod::Near);
                assert_closed(&m, 99.5);
                let walls = area * (106.0 - 99.5);
                let v = volume(&m, 99.5);
                assert!(
                    v > walls && v < walls + area * 3.2,
                    "{i} {shape:?}: {v} vs {walls}"
                );
                let (lo, hi) = y_range(&m);
                assert_eq!(lo, 99.5);
                assert!(
                    hi > 106.0 + 1.0 && hi < 106.0 + 3.2 * 1.5,
                    "{i} {shape:?}: {hi}"
                );
            }
        }
        // The L (8 m arms): hipped to 3.2 m at (4, 4), (16, 4), (4, 16).
        let m = draw(
            &l_house(),
            &skeleton_spec(RoofShape::Hipped, 3.2),
            Lod::Near,
        );
        for (x, z) in [
            (4.0, 4.0),
            (16.0, 4.0),
            (4.0, 16.0),
            (10.0, 4.0),
            (4.0, 10.0),
        ] {
            let y = top_at(&m, x, z).unwrap();
            assert!((y - 109.2).abs() < 1e-3, "({x}, {z}): {y}");
        }
        assert!((top_at(&m, 19.0, 4.0).unwrap() - (106.0 + 0.8)).abs() < 1e-3);
        // Gabled: the ridges run out to the arm ends' walls.
        let m = draw(
            &l_house(),
            &skeleton_spec(RoofShape::Gabled, 3.2),
            Lod::Near,
        );
        assert!((top_at(&m, 19.99, 4.0).unwrap() - 109.2).abs() < 1e-2);
        assert!((top_at(&m, 4.0, 19.99).unwrap() - 109.2).abs() < 1e-2);
        // The gables are walls: vertical faces with the upper floors' paint.
        let gable_tris = m
            .normals
            .iter()
            .zip(&m.positions)
            .filter(|(n, p)| n[1].abs() < 1e-6 && p[1] > 106.5)
            .count();
        assert!(gable_tris >= 2);
    }

    /// The overhanging near roof and the mid and far roofs inside it.
    #[test]
    fn skeleton_roofs_nest_their_levels_of_detail() {
        let b = l_house();
        let h = HouseSpec {
            overhang: 0.6,
            ..skeleton_spec(RoofShape::Hipped, 3.2)
        };
        let near = draw(&b, &h, Lod::Near);
        let mid = draw(&b, &h, Lod::Mid);
        let far = draw(&b, &h, Lod::Far);
        let mut k = 0;
        for x in (1..20).map(|v| v as f32 - 0.25) {
            for z in (1..20).map(|v| v as f32 - 0.25) {
                let (Some(n), Some(m)) = (top_at(&near, x, z), top_at(&mid, x, z)) else {
                    continue;
                };
                assert!(m < n, "mid above near at ({x}, {z}): {m} {n}");
                if let Some(f) = top_at(&far, x, z) {
                    assert!(f < m, "far above mid at ({x}, {z})");
                }
                k += 1;
            }
        }
        assert!(k > 150, "{k}");
        // The near roof overhangs the walls (eaves lowered along the pitch).
        let (lo_x, _) = near
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, c), p| (a.min(p[0]), c.max(p[0])));
        assert!((lo_x + 0.6).abs() < 1e-3, "{lo_x}");
        let (lo_x, _) = mid
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, c), p| (a.min(p[0]), c.max(p[0])));
        assert!((lo_x - MID_INSET_M).abs() < 1e-4);
        // A chimney rises above the ridge in the near level only.
        let c = HouseSpec { chimney: true, ..h };
        assert!(y_range(&draw(&b, &c, Lod::Near)).1 > 109.2 + 0.5);
        assert!(y_range(&draw(&b, &c, Lod::Mid)).1 < 109.2);
    }

    /// Footprints without a skeleton (self-intersecting, too many points)
    /// get a flat roof at the eaves.
    #[test]
    fn skeleton_roofs_fall_back_to_flat() {
        let bow = polygon(
            vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]],
            vec![],
        );
        let many = polygon(
            (0..80)
                .map(|i| {
                    let a = i as f32 / 80.0 * std::f32::consts::TAU;
                    let r = if i % 2 == 0 { 10.0 } else { 9.0 };
                    [r * a.cos(), r * a.sin()]
                })
                .collect(),
            vec![],
        );
        for b in [bow, many] {
            for lod in [Lod::Near, Lod::Mid] {
                let m = draw(&b, &skeleton_spec(RoofShape::Gabled, 3.2), lod);
                assert!(!m.indices.is_empty());
                assert!(y_range(&m).1 <= 106.0 + 1e-4, "{lod:?}");
            }
        }
    }

    fn dormer_house(kind: Dormer, roof_h: f32) -> (Building, HouseSpec) {
        let b = building(Roof::Gabled { height_m: roof_h });
        let h = HouseSpec {
            dormers: Some(kind),
            ..spec(RoofShape::Gabled, roof_h)
        };
        (b, h)
    }

    /// Dormers (spec §14 Phase 8 T4, v3.26): planned on the long sides
    /// where the roof has room, geometry with a window near, the shader's
    /// roof detail (code 15) mid, nothing far.
    #[test]
    fn dormers_sit_on_the_long_sides_where_they_fit() {
        let (b, h) = dormer_house(Dormer::Gabled, 4.5);
        let rows = rect_rows(&b.outline, &h, Dormer::Gabled);
        assert_eq!(rows.len(), 2, "{rows:?}");
        for (_, r) in &rows {
            assert_eq!((r.count, r.spacing_dm, r.up_dm), (3, 34, 18));
            assert!((r.front - 1.2).abs() < 0.01);
            // The cells lie on the 12 m eave, centred.
            let (c0, c2) = (r.centre(0), r.centre(2));
            assert!((c0.distance(c2) - 6.8).abs() < 1e-4);
            assert!((c0.y - 106.0).abs() < 1e-5);
        }
        // Near: three dormers a side with a window each, closed solids.
        let near = draw(&b, &h, Lod::Near);
        let plain = draw(&b, &HouseSpec { dormers: None, ..h }, Lod::Near);
        assert!(near.indices.len() > plain.indices.len());
        let windows_up: Vec<f32> = near
            .positions
            .iter()
            .zip(&near.uvs_b)
            .filter(|(p, u)| decode_uv_b(**u).2 > 0 && p[1] > 107.0)
            .map(|(p, _)| p[0])
            .collect();
        assert!(!windows_up.is_empty());
        assert!(
            plain
                .positions
                .iter()
                .zip(&plain.uvs_b)
                .all(|(p, u)| decode_uv_b(*u).2 == 0 || p[1] <= 106.0 + 1e-4)
        );
        // Mid: the roof faces carry the detail.
        let mid = draw(&b, &h, Lod::Mid);
        let codes: std::collections::BTreeSet<(u32, u32)> = mid
            .uvs_b
            .iter()
            .map(|u| decode_uv_b(*u))
            .filter(|d| d.2 == ROOF_DETAIL)
            .map(|d| (d.4, (d.3 * 10.0).round() as u32))
            .collect();
        assert_eq!(codes.len(), 1, "{codes:?}");
        let (levels, up) = *codes.iter().next().unwrap();
        assert_eq!(up, 18);
        assert_eq!(
            (levels % 4, (levels / 4) % 16, levels / 64),
            (dormer_code(Dormer::Gabled), 3, 34)
        );
        assert!(
            draw(&b, &h, Lod::Far)
                .uvs_b
                .iter()
                .all(|u| decode_uv_b(*u).2 != ROOF_DETAIL)
        );
        // Shed dormers and roof windows; a low roof has no room.
        let (b, h) = dormer_house(Dormer::Shed, 4.5);
        assert!(!rect_rows(&b.outline, &h, Dormer::Shed).is_empty());
        let (b, h) = dormer_house(Dormer::Window, 3.0);
        let rows = rect_rows(&b.outline, &h, Dormer::Window);
        assert!(!rows.is_empty());
        let near = draw(&b, &h, Lod::Near);
        assert!(
            near.uvs_b
                .iter()
                .any(|u| decode_uv_b(*u).0 == Layer::Glass as u32)
        );
        let (b, h) = dormer_house(Dormer::Gabled, 1.2);
        assert!(rect_rows(&b.outline, &h, Dormer::Gabled).is_empty());
        // Skeleton roofs: on the L's long faces.
        let sk = roof_skeleton(&l_house().outline, &[], true).unwrap().0;
        let rows = skeleton_rows(&sk, 106.0, 4.5 / 4.0, Dormer::Gabled);
        assert!(rows.len() >= 2, "{rows:?}");
        assert!(rows.iter().all(|(e, _)| !sk.gable[*e]));
        let l = HouseSpec {
            dormers: Some(Dormer::Gabled),
            ..skeleton_spec(RoofShape::Gabled, 4.5)
        };
        let mid = draw(&l_house(), &l, Lod::Mid);
        assert!(mid.uvs_b.iter().any(|u| decode_uv_b(*u).2 == ROOF_DETAIL));
    }

    #[test]
    fn plan_row_follows_the_face() {
        // A hip face (trapezoid): narrower up the slope.
        let trap = [[0.0, 0.0], [16.0, 0.0], [12.0, 4.0], [4.0, 4.0]];
        let (start, front, _, n, sp) = plan_row(&trap, 1.0, Dormer::Window).unwrap();
        let cells = n as f32 * sp as f32 / 10.0;
        assert!(start >= front + 0.4 - 1e-4 && start + cells <= 16.0 - front - 0.4 + 1e-4);
        assert!(plan_row(&trap, 0.2, Dormer::Window).is_none(), "too flat");
        assert!(plan_row(&[[0.0, 0.0], [3.0, 0.0], [1.5, 1.5]], 1.0, Dormer::Gabled).is_none());
        assert_eq!(section(&trap, 2.0), vec![(2.0, 14.0)]);
    }

    /// Balconies (v3.26): along the chosen wall on the upper floors, near
    /// only; the gable end facing south the most, the long side the south one.
    #[test]
    fn balconies_run_along_their_wall_near_only() {
        let b = building(Roof::Gabled { height_m: 3.0 });
        let h = HouseSpec {
            balcony: Some(BalconySide::Long),
            ..spec(RoofShape::Gabled, 3.0)
        };
        let near = draw(&b, &h, Lod::Near);
        let out: Vec<&[f32; 3]> = near.positions.iter().filter(|p| p[2] > 8.7).collect();
        assert!(!out.is_empty(), "the balcony in front of the south wall");
        assert!(out.iter().all(|p| p[2] <= 8.0 + BALCONY_DEPTH_M + 1e-4));
        let ys: Vec<f32> = out.iter().map(|p| p[1]).collect();
        let lo = ys.iter().copied().fold(f32::MAX, f32::min);
        let hi = ys.iter().copied().fold(f32::MIN, f32::max);
        // Floor of the first storey at 103 (ground 100, 3 m storeys).
        assert!(
            (lo - (103.0 - 0.14)).abs() < 1e-4 && (hi - 104.0).abs() < 1e-4,
            "{lo} {hi}"
        );
        assert!(draw(&b, &h, Lod::Mid).positions.iter().all(|p| p[2] <= 8.0));
        // A gable balcony: off one of the gable walls (x = 0 or 12).
        let g = HouseSpec {
            balcony: Some(BalconySide::Gable),
            ..h
        };
        let near = draw(&b, &g, Lod::Near);
        assert!(near.positions.iter().any(|p| p[0] > 12.5 || p[0] < -0.5));
        assert_eq!(
            balcony_wall(&b.outline, BalconySide::Long, &[1, 3]),
            Some(2)
        );
        assert!(matches!(
            balcony_wall(&b.outline, BalconySide::Gable, &[1, 3]),
            Some(1 | 3)
        ));
        // One storey: no balcony.
        let low = HouseSpec {
            levels: 1,
            storey_h: 6.0,
            ..h
        };
        let one = draw(&b, &low, Lod::Near);
        assert!(one.positions.iter().all(|p| p[2] <= 8.0 + 0.7));
    }

    /// `building:part` (v3.26): parts covering their building are drawn
    /// instead of it, each with its own height and roof; parts that do
    /// not cover it stand on top of it, a little inside.
    #[test]
    fn parts_replace_the_outline_they_cover() {
        use ocw_world::buildings::BuildingParts;
        let outline = Building {
            outline: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 12.0], [0.0, 12.0]],
            roof: Roof::Flat,
            eave_y: 112.0,
            ..building(Roof::Flat)
        };
        let nave = Building {
            outline: vec![[6.0, 0.0], [30.0, 0.0], [30.0, 12.0], [6.0, 12.0]],
            eave_y: 110.0,
            ..outline.clone()
        };
        let tower = Building {
            outline: vec![[0.0, 3.0], [6.0, 3.0], [6.0, 9.0], [0.0, 9.0]],
            roof: Roof::Gabled { height_m: 10.0 },
            eave_y: 125.0,
            ..outline.clone()
        };
        let church = Building {
            parts: BuildingParts {
                list: vec![nave, tower],
                cover: true,
            },
            ..outline.clone()
        };
        let flat = HouseSpec {
            eave_y: 112.0,
            parapet: 0.0,
            ..spec(RoofShape::Flat, 0.0)
        };
        let specs = TileSpecs {
            specs: vec![flat],
            parts: vec![vec![
                HouseSpec {
                    eave_y: 110.0,
                    ..flat
                },
                HouseSpec {
                    eave_y: 125.0,
                    overhang: 0.0,
                    ..spec(RoofShape::Pyramidal, 10.0)
                },
            ]],
        };
        assert_eq!(specs.top(0), 135.0);
        let m = tile_houses(std::slice::from_ref(&church), &specs, [0.0, 0.0], 250.0);
        let near = &m.near[0];
        let ys: Vec<f32> = near
            .mesh
            .positions
            .iter()
            .map(|p| p[1] + near.origin[1])
            .collect();
        assert!(ys.iter().any(|y| (y - 135.0).abs() < 1e-3), "the spire");
        assert!(!ys.iter().any(|y| (y - 112.0).abs() < 1e-3), "no outline");
        // Not covered: the outline and the parts, inside it.
        let open = Building {
            parts: BuildingParts {
                cover: false,
                ..church.parts.clone()
            },
            ..church.clone()
        };
        let m = tile_houses(std::slice::from_ref(&open), &specs, [0.0, 0.0], 250.0);
        let near = &m.near[0];
        let pts: Vec<Vec3> = near
            .mesh
            .positions
            .iter()
            .map(|p| Vec3::from(*p) + Vec3::from(near.origin))
            .collect();
        assert!(pts.iter().any(|p| (p.y - 112.0).abs() < 1e-3));
        let tower_lo_x = pts
            .iter()
            .filter(|p| p.y > 113.0)
            .map(|p| p.x)
            .fold(f32::MAX, f32::min);
        assert!((tower_lo_x - PART_INSET_M).abs() < 1e-3, "{tower_lo_x}");
        // The far level keeps the spire (by the building's size).
        let far = m.far.as_ref().unwrap();
        assert!(
            far.mesh
                .positions
                .iter()
                .any(|p| p[1] + far.origin[1] > 130.0)
        );
    }
}
