//! Road strip meshes (spec §4.4, §4.6 step 6, §14 Phase 8 T5): the
//! carriageway with its marking code, kerbs and sidewalks in towns, the
//! shoulder that blends the road edge into the terrain, retaining walls
//! where the terrain falls away, and the overlays of the crossings, stop
//! and give-way lines.
//!
//! Every vertex carries what `road.wesl` needs:
//! - `uv` = (across, along) in metres: across from the road's centre (+
//!   right of the strip's direction), along the road edge's arc length
//!   (dashes, wear and textures run on across tiles);
//! - `uv_b.x` = layer + 8 · [`Kind`] + 64 · extra (cycle lanes, track),
//!   `uv_b.y` = the strip's lines code ([`MarkCode::lines_code`]); an
//!   overlay's `uv_b.y` is its crossing kind + 4 · half;
//! - the colour: rgb the far colour, alpha the road's half width.
//!
//! **Road edge (the "slab" fix):** the road surface is drawn
//! [`crate::ROAD_SURFACE_LIFT_M`] above the profile and the terrain is
//! settled at least 5 cm below it (§4.8.3), so on flat ground a vertical
//! edge face showed as a light slab edge above the terrain (Las Cañadas).
//! Now a short lip (3 cm) leads to a sloping **shoulder** – a verge of the
//! shoulder material – that ends just under the terrain surface 0.4 m to
//! 2.7 m beside the road, depending on how far the ground lies below the
//! edge (1 : 1.5); a skirt below its outer edge closes any gap. Where the
//! ground lies more than 1.5 m below the edge (the upper leg of a
//! hairpin, a road above another), a retaining wall goes down to the
//! ground as before. Riders still stand on the strip surface (spec 2.47).

use bevy::prelude::*;
use ocw_world::RoadSurface;
use ocw_world::package::RoadStrip;
use ocw_world::road_dress::{CrossingKind, KERB_LAYER, MarkCode, MarkingStyle, SIDEWALK_LAYER};

use crate::package::MeshArrays;

/// Plain skirt depth below the road surface where there is no terrain.
pub const EDGE_DEPTH_M: f32 = 2.0;
/// Deepest retaining wall drawn under a road edge (m).
pub const MAX_WALL_M: f32 = 40.0;
/// The shoulder starts this far below the road surface (a lip; the
/// surface of a crossing road at a junction covers it).
pub const SHOULDER_STEP_M: f32 = 0.03;
/// Narrowest shoulder (m).
pub const SHOULDER_MIN_M: f32 = 0.4;
/// Horizontal metres of shoulder per metre the ground lies below the edge.
pub const SHOULDER_SLOPE: f32 = 1.5;
/// Deeper than this below the edge, the ground gets a wall, not a verge.
pub const SHOULDER_MAX_DROP_M: f32 = 1.5;
/// The shoulder ends this far under the terrain surface.
pub const SHOULDER_SINK_M: f32 = 0.04;
/// The skirt hangs this far below the shoulder's outer edge.
pub const SKIRT_M: f32 = 0.3;
/// Overlays (crossings) lie this far above the road surface.
pub const OVERLAY_LIFT_M: f32 = 0.012;
/// Overlays reach this far beyond their painted length (the filtered
/// edges of the paint).
pub const OVERLAY_MARGIN_M: f32 = 0.25;
use ocw_world::road_dress::SIDEWALK_FALL;

/// What a vertex belongs to (`uv_b.x / 8 % 8` in the shader).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Surface = 0,
    /// Lip, skirts and retaining walls.
    Wall = 1,
    Shoulder = 2,
    Kerb = 3,
    Sidewalk = 4,
    /// A transverse marking overlay.
    Crossing = 5,
}

/// The first extra texture coordinate of a vertex.
pub fn code_b(layer: u8, kind: Kind, extra: u32) -> f32 {
    (u32::from(layer) + 8 * kind as u32 + 64 * extra) as f32
}

/// Decodes [`code_b`]: (layer, kind, extra).
pub fn decode_b(x: f32) -> (u8, u8, u32) {
    let c = x.round() as u32;
    ((c & 7) as u8, (c >> 3 & 7) as u8, c >> 6)
}

fn lin(r: f32, g: f32, b: f32) -> [f32; 3] {
    [r * r, g * g, b * b]
}

/// Flat colours of the road parts where no material library gives their
/// means (linear).
pub const EDGE_COLOUR: [f32; 3] = [0.2025, 0.1849, 0.1444];
pub const SIDEWALK_COLOUR: [f32; 3] = [0.25, 0.24, 0.22];
pub const KERB_COLOUR: [f32; 3] = [0.36, 0.35, 0.33];

/// The shoulder beside a road edge at `top` (the edge of the drawn
/// surface) going out along `out`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shoulder {
    /// Horizontal width (m).
    pub width: f32,
    /// Height of its outer edge.
    pub outer_y: f32,
    /// Bottom of the skirt or wall below the outer edge.
    pub bottom_y: f32,
    /// The ground lies too far below: a retaining wall.
    pub wall: bool,
}

/// The shoulder at a road edge: `ground` is the terrain surface height at
/// (x, z) (`None` beyond the package's tiles), `narrow` keeps it at its
/// narrowest (junction zones, where other roads come in).
pub fn shoulder(
    top: Vec3,
    out: Vec3,
    ground: &dyn Fn(f32, f32) -> Option<f32>,
    narrow: bool,
) -> Shoulder {
    let lip = top.y - SHOULDER_STEP_M;
    let Some(g0) = ground(top.x, top.z) else {
        return Shoulder {
            width: SHOULDER_MIN_M,
            outer_y: lip - 0.1,
            bottom_y: top.y - EDGE_DEPTH_M,
            wall: false,
        };
    };
    let drop = top.y - g0;
    if drop > SHOULDER_MAX_DROP_M {
        // A wall down to below the ground at the edge (and no deeper than
        // the deepest wall).
        let o = top + out * SHOULDER_MIN_M;
        let g = ground(o.x, o.z).unwrap_or(g0).min(g0);
        let bottom = (g - SKIRT_M).max(top.y - MAX_WALL_M);
        return Shoulder {
            width: SHOULDER_MIN_M,
            outer_y: lip - 0.05,
            bottom_y: bottom.min(lip - 0.05 - SKIRT_M),
            wall: true,
        };
    }
    let width = if narrow {
        SHOULDER_MIN_M
    } else {
        SHOULDER_MIN_M + SHOULDER_SLOPE * drop.max(0.0)
    };
    let o = top + out * width;
    let g = ground(o.x, o.z).unwrap_or(g0);
    let outer_y = (g - SHOULDER_SINK_M).min(lip);
    Shoulder {
        width,
        outer_y,
        bottom_y: outer_y.min(g) - SKIRT_M,
        wall: false,
    }
}

/// How the strips of a tile are drawn: the colours of the library's road
/// layers (by layer, linear; the far colour), the terrain, the bridge
/// decks.
pub struct RoadContext<'a> {
    pub colours: Option<&'a [[f32; 3]]>,
    pub ground: &'a dyn Fn(f32, f32) -> Option<f32>,
    /// Whether a point of a road edge lies on a bridge deck (no shoulder,
    /// no skirt, no sidewalk: the deck has its own sides).
    pub deck: &'a dyn Fn([f32; 3]) -> bool,
    pub style: &'a MarkingStyle,
}

impl RoadContext<'_> {
    pub(crate) fn colour(&self, layer: u8, fallback: [f32; 3]) -> [f32; 3] {
        self.colours
            .and_then(|c| c.get(usize::from(layer)))
            .copied()
            .unwrap_or(fallback)
    }
}

/// The road meshes of a tile: surfaces with kerbs, sidewalks, shoulders
/// and walls (opaque), and the transverse marking overlays (blended).
#[derive(Default)]
pub struct RoadMeshes {
    pub roads: MeshArrays,
    pub overlays: MeshArrays,
}

/// The direction of a strip at point `i` (horizontal, unit).
fn direction(p: &[Vec3], i: usize) -> Vec3 {
    let n = p.len();
    (p[(i + 1).min(n - 1)] - p[i.saturating_sub(1)])
        .with_y(0.0)
        .normalize_or_zero()
}

/// Pushes one vertex.
fn vertex(
    m: &mut MeshArrays,
    p: Vec3,
    n: Vec3,
    colour: [f32; 3],
    half: f32,
    uv: [f32; 2],
    b: [f32; 2],
) {
    m.positions.push(p.to_array());
    m.normals.push(n.to_array());
    m.colors.push([colour[0], colour[1], colour[2], half]);
    m.uvs.push(uv);
    m.uvs_b.push(b);
}

/// Quads between two vertex rows of `n` vertices each, starting at `a` and
/// `b` (row k: a + k, b + k); `both`: also the back faces.
fn ribbon(m: &mut MeshArrays, a: u32, b: u32, n: u32, both: bool) {
    for k in 0..n.saturating_sub(1) {
        let (a0, a1, b0, b1) = (a + k, a + k + 1, b + k, b + k + 1);
        m.indices.extend([a0, b0, a1, b0, b1, a1]);
        if both {
            m.indices.extend([a0, a1, b0, b0, a1, b1]);
        }
    }
}

/// The layer of a strip's shoulder: gravel beside paved roads, the road's
/// own material beside unpaved ones.
fn shoulder_layer(surface: RoadSurface) -> u8 {
    match surface {
        RoadSurface::Asphalt | RoadSurface::WornAsphalt | RoadSurface::Paving => {
            RoadSurface::Gravel.code()
        }
        s => s.code(),
    }
}

/// The road meshes of `strips` (a tile's, in its local metres).
/// Where two sidewalks meet at a junction the corner is rounded
/// (`road_corners`): the strips' sidewalks end square at the arc.
pub fn road_meshes(strips: &[RoadStrip], ctx: &RoadContext<'_>) -> RoadMeshes {
    let mut out = RoadMeshes::default();
    let corners = crate::road_corners::corners(strips);
    let trims = crate::road_corners::trims(&corners, strips.len());
    for (s, trim) in strips.iter().zip(&trims) {
        if trim.iter().flatten().any(Option::is_some) {
            let cut = crate::road_corners::with_cuts(s, trim);
            strip_mesh(&mut out.roads, &cut, ctx, trim);
        } else {
            strip_mesh(&mut out.roads, s, ctx, trim);
        }
        crossing_overlays(&mut out.overlays, s, ctx);
    }
    for c in &corners {
        crate::road_corners::corner_mesh(&mut out.roads, c, strips, ctx);
    }
    out
}

fn strip_mesh(
    m: &mut MeshArrays,
    s: &RoadStrip,
    ctx: &RoadContext<'_>,
    trim: &[[Option<f32>; 2]; 2],
) {
    let p: Vec<Vec3> = s
        .points
        .iter()
        .map(|q| Vec3::from(*q) + Vec3::Y * crate::ROAD_SURFACE_LIFT_M)
        .collect();
    let n = p.len();
    if n < 2 {
        return;
    }
    let half = s.width_m / 2.0;
    let layer = s.surface.code();
    let code = MarkCode::of(&s.dress, s.class, s.surface);
    let lines = code.lines_code() as f32;
    let extra = code.extra_code();
    let far = ctx.colour(layer, {
        let c = crate::package::road_colour(s.class, s.surface);
        [c[0], c[1], c[2]]
    });
    // Arc length along the drawn points from the edge's start.
    let mut along = vec![s.dress.start_m; n];
    let mut local = vec![0.0f32; n];
    for i in 1..n {
        local[i] = local[i - 1] + (p[i] - p[i - 1]).length();
        along[i] = s.dress.start_m + local[i];
    }
    let rights: Vec<Vec3> = (0..n)
        .map(|i| {
            let d = direction(&p, i);
            Vec3::new(-d.z, 0.0, d.x)
        })
        .collect();
    // The carriageway.
    let base = m.positions.len() as u32;
    for i in 0..n {
        for side in [-half, half] {
            vertex(
                m,
                p[i] + rights[i] * side,
                Vec3::Y,
                far,
                half,
                [side, along[i]],
                [code_b(layer, Kind::Surface, extra), lines],
            );
        }
    }
    for i in 0..n as u32 - 1 {
        let (l0, r0, l1, r1) = (
            base + 2 * i,
            base + 2 * i + 1,
            base + 2 * i + 2,
            base + 2 * i + 3,
        );
        m.indices.extend([l0, r0, l1, r0, r1, l1]);
    }
    let decks: Vec<[bool; 2]> = (0..n)
        .map(|i| {
            [-1.0f32, 1.0].map(|side| {
                let top = p[i] + rights[i] * side * half;
                (ctx.deck)([top.x, top.y - crate::ROAD_SURFACE_LIFT_M, top.z])
            })
        })
        .collect();
    let shoulder_colour = ctx.colour(shoulder_layer(s.surface), lin(0.55, 0.5, 0.42));
    let wall_colour = ctx.colour(KERB_LAYER, EDGE_COLOUR);
    let sidewalk_layer = if s.dress.sidewalk_asphalt {
        RoadSurface::Asphalt.code()
    } else {
        SIDEWALK_LAYER
    };
    let sidewalk_colour = ctx.colour(sidewalk_layer, SIDEWALK_COLOUR);
    let kerb_colour = ctx.colour(KERB_LAYER, KERB_COLOUR);
    for (si, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        let sidewalk = s.dress.sidewalk_m[si] > 0.0;
        // Per point: the edge top beyond the sidewalk (where the shoulder
        // starts) and the sidewalk's width and kerb height.
        let mut tops = Vec::with_capacity(n);
        let mut kerbs = Vec::with_capacity(n);
        for i in 0..n {
            let out = rights[i] * side;
            let edge = p[i] + out * half;
            let (w, k) = if sidewalk && !decks[i][si] {
                crate::road_corners::kerb_trimmed(&s.dress, si, local[i], local[n - 1], &trim[si])
            } else {
                (0.0, 0.0)
            };
            kerbs.push((w, k));
            tops.push(edge + out * w + Vec3::Y * (k + w * SIDEWALK_FALL));
        }
        if sidewalk {
            // Kerb face (towards the road), then the sidewalk surface; the
            // shader paints the kerb stones' top on its first centimetres.
            let kb = m.positions.len() as u32;
            for i in 0..n {
                let out = rights[i] * side;
                let edge = p[i] + out * half;
                let k = kerbs[i].1;
                for (y, u) in [(0.0, side * half), (k, side * (half - k))] {
                    vertex(
                        m,
                        edge + Vec3::Y * y,
                        -out,
                        kerb_colour,
                        half,
                        [u, along[i]],
                        [code_b(KERB_LAYER, Kind::Kerb, 0), 0.0],
                    );
                }
            }
            // Bottom row then top row, interleaved: rows by stride 2.
            for i in 0..n as u32 - 1 {
                let (b0, t0, b1, t1) = (kb + 2 * i, kb + 2 * i + 1, kb + 2 * i + 2, kb + 2 * i + 3);
                if side > 0.0 {
                    m.indices.extend([b0, t0, b1, t0, t1, b1]);
                } else {
                    m.indices.extend([b0, b1, t0, t0, b1, t1]);
                }
            }
            let sb = m.positions.len() as u32;
            for i in 0..n {
                let out = rights[i] * side;
                let (w, k) = kerbs[i];
                let edge = p[i] + out * half + Vec3::Y * k;
                for (q, u) in [(edge, side * half), (tops[i], side * (half + w))] {
                    vertex(
                        m,
                        q,
                        Vec3::Y,
                        sidewalk_colour,
                        half,
                        [u, along[i]],
                        [code_b(sidewalk_layer, Kind::Sidewalk, 0), 0.0],
                    );
                }
            }
            for i in 0..n as u32 - 1 {
                let (a0, b0, a1, b1) = (sb + 2 * i, sb + 2 * i + 1, sb + 2 * i + 2, sb + 2 * i + 3);
                if side > 0.0 {
                    m.indices.extend([a0, b0, a1, b0, b1, a1]);
                } else {
                    m.indices.extend([a0, a1, b0, b0, a1, b1]);
                }
            }
        }
        // Lip, shoulder and skirt (nothing on a bridge deck).
        let rows = m.positions.len() as u32;
        let mut shoulders = Vec::with_capacity(n);
        for i in 0..n {
            let out = rights[i] * side;
            let top = tops[i];
            let sh = if decks[i][si] {
                Shoulder {
                    width: 0.0,
                    outer_y: top.y,
                    bottom_y: top.y,
                    wall: false,
                }
            } else {
                shoulder(top, out, ctx.ground, s.dress.in_junction(local[i]))
            };
            shoulders.push(sh);
        }
        let e0 = side * half;
        // Row 0: top; row 1: lip; row 2: shoulder outer; row 3: skirt bottom.
        for row in 0..4 {
            for i in 0..n {
                let out = rights[i] * side;
                let top = tops[i];
                let sh = shoulders[i];
                let lip = if decks[i][si] {
                    top.y
                } else {
                    top.y - SHOULDER_STEP_M
                };
                let e = e0 + side * kerbs[i].0;
                let outer = top + out * sh.width;
                let (q, u, kind, layer, colour, normal) = match row {
                    0 => (top, e, Kind::Wall, KERB_LAYER, wall_colour, out),
                    1 => (
                        top.with_y(lip),
                        e + side * 0.03,
                        Kind::Wall,
                        KERB_LAYER,
                        wall_colour,
                        out,
                    ),
                    2 => {
                        let n =
                            (out * (lip - sh.outer_y) + Vec3::Y * sh.width.max(1e-3)).normalize();
                        if sh.wall {
                            (
                                outer.with_y(sh.outer_y),
                                e + side * sh.width,
                                Kind::Wall,
                                KERB_LAYER,
                                wall_colour,
                                n,
                            )
                        } else {
                            (
                                outer.with_y(sh.outer_y),
                                e + side * sh.width,
                                Kind::Shoulder,
                                shoulder_layer(s.surface),
                                shoulder_colour,
                                n,
                            )
                        }
                    }
                    _ => (
                        outer.with_y(sh.bottom_y),
                        e + side * (sh.width + sh.outer_y - sh.bottom_y),
                        Kind::Wall,
                        KERB_LAYER,
                        wall_colour,
                        out,
                    ),
                };
                vertex(
                    m,
                    q,
                    normal,
                    colour,
                    half,
                    [u, along[i]],
                    [code_b(layer, kind, 0), 0.0],
                );
            }
        }
        // The lip faces out; the shoulder up; the skirt both ways (seen
        // from below a hairpin's upper leg too). Inner-edge vertices of the
        // shoulder are the lip's bottom row with the wall kind: the
        // shoulder gets its own copy for its material.
        let nn = n as u32;
        let (r0, r1, r2, r3) = (rows, rows + nn, rows + 2 * nn, rows + 3 * nn);
        // Row 1 (the lip's bottom) closes the lip; the shoulder starts
        // from its own copy of that row below.
        let face = |m: &mut MeshArrays, a: u32, b: u32, both: bool| {
            if side > 0.0 {
                ribbon(m, a, b, nn, both);
            } else {
                ribbon(m, b, a, nn, both);
            }
        };
        face(m, r0, r1, false);
        // The shoulder's inner row in its own material.
        let si_row = m.positions.len() as u32;
        for i in 0..n {
            let top = tops[i];
            let sh = shoulders[i];
            let lip = if decks[i][si] {
                top.y
            } else {
                top.y - SHOULDER_STEP_M
            };
            let out = rights[i] * side;
            let nrm = (out * (lip - sh.outer_y) + Vec3::Y * sh.width.max(1e-3)).normalize();
            let e = e0 + side * (kerbs[i].0 + 0.03);
            let (layer, kind, colour) = if sh.wall {
                (KERB_LAYER, Kind::Wall, wall_colour)
            } else {
                (shoulder_layer(s.surface), Kind::Shoulder, shoulder_colour)
            };
            vertex(
                m,
                top.with_y(lip),
                nrm,
                colour,
                half,
                [e, along[i]],
                [code_b(layer, kind, 0), 0.0],
            );
        }
        face(m, si_row, r2, false);
        face(m, r2, r3, true);
    }
}

/// A point at arc length `s` along a strip (with its cumulative lengths):
/// position and horizontal direction.
fn at_along(p: &[Vec3], cum: &[f32], s: f32) -> (Vec3, Vec3) {
    let n = p.len();
    let k = cum.partition_point(|&c| c <= s).clamp(1, n - 1);
    let (a, b) = (p[k - 1], p[k]);
    let l = (cum[k] - cum[k - 1]).max(1e-6);
    let t = ((s - cum[k - 1]) / l).clamp(-1.0, 2.0);
    (a + (b - a) * t, (b - a).with_y(0.0).normalize_or_zero())
}

/// The overlays of a strip's crossings: short ribbons on the road surface
/// over the painted length (plus a margin), uv = (across, metres from the
/// start of the painted length).
fn crossing_overlays(m: &mut MeshArrays, s: &RoadStrip, ctx: &RoadContext<'_>) {
    if s.dress.crossings.is_empty()
        || !matches!(s.surface, RoadSurface::Asphalt | RoadSurface::WornAsphalt)
    {
        return;
    }
    let p: Vec<Vec3> = s
        .points
        .iter()
        .map(|q| Vec3::from(*q) + Vec3::Y * (crate::ROAD_SURFACE_LIFT_M + OVERLAY_LIFT_M))
        .collect();
    if p.len() < 2 {
        return;
    }
    let mut cum = vec![0.0f32; p.len()];
    for i in 1..p.len() {
        cum[i] = cum[i - 1] + (p[i] - p[i - 1]).length();
    }
    let half = s.width_m / 2.0;
    for c in &s.dress.crossings {
        let len = ctx.style.crossing_length_m(c.kind);
        let start = c.along_m - len / 2.0;
        let steps = ((len + 2.0 * OVERLAY_MARGIN_M) / 1.0).ceil().max(1.0) as usize;
        let base = m.positions.len() as u32;
        let colour = if c.kind == CrossingKind::Zebra {
            ctx.style.crossing_colour
        } else {
            ctx.style.line_colour
        };
        for j in 0..=steps {
            let t = -OVERLAY_MARGIN_M + (len + 2.0 * OVERLAY_MARGIN_M) * j as f32 / steps as f32;
            let (q, d) = at_along(&p, &cum, start + t);
            let right = Vec3::new(-d.z, 0.0, d.x);
            for side in [-half, half] {
                vertex(
                    m,
                    q + right * side,
                    Vec3::Y,
                    colour,
                    half,
                    [side, t],
                    [
                        code_b(0, Kind::Crossing, 0),
                        f32::from(c.kind as u8 + 4 * c.half as u8),
                    ],
                );
            }
        }
        for j in 0..steps as u32 {
            let (l0, r0, l1, r1) = (
                base + 2 * j,
                base + 2 * j + 1,
                base + 2 * j + 2,
                base + 2 * j + 3,
            );
            m.indices.extend([l0, r0, l1, r0, r1, l1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::RoadDress;
    use ocw_world::graph::RoadClass;
    use ocw_world::road_dress::{CentreLine, Crossing, Half};

    fn strip(class: RoadClass, width: f32, y: f32, dress: RoadDress) -> RoadStrip {
        RoadStrip {
            class,
            width_m: width,
            points: (0..11).map(|i| [i as f32 * 10.0, y, 50.0]).collect(),
            surface: RoadSurface::Asphalt,
            dress,
            lit: ocw_world::RoadLit::Unknown,
        }
    }

    fn style() -> MarkingStyle {
        MarkingStyle::default()
    }

    fn flat(h: f32) -> impl Fn(f32, f32) -> Option<f32> {
        move |_, _| Some(h)
    }

    fn meshes(strips: &[RoadStrip], ground: &dyn Fn(f32, f32) -> Option<f32>) -> RoadMeshes {
        let st = style();
        road_meshes(
            strips,
            &RoadContext {
                colours: None,
                ground,
                deck: &|_| false,
                style: &st,
            },
        )
    }

    /// The defect "road above the ground" (Las Cañadas): on a flat plain
    /// with the terrain settled 8 cm below the drawn surface, nothing of
    /// the road edge stands above the terrain beyond the lip: the shoulder
    /// slopes from 3 cm below the surface to under the terrain, and no
    /// vertical face shows more than the lip.
    #[test]
    fn the_road_edge_blends_into_flat_terrain() {
        let road_y = 100.0f32;
        let terrain = road_y + crate::ROAD_SURFACE_LIFT_M - 0.08;
        let g = flat(terrain);
        let s = strip(
            RoadClass::Secondary,
            6.0,
            road_y,
            RoadDress::of_class(RoadClass::Secondary, 6.0),
        );
        let m = meshes(&[s], &g).roads;
        let top = road_y + crate::ROAD_SURFACE_LIFT_M;
        for (q, b) in m.positions.iter().zip(&m.uvs_b) {
            let (_, kind, _) = decode_b(b[0]);
            let beside = (q[2] - 50.0).abs() > 3.0 + 1e-3;
            if beside {
                // Beyond the edge every vertex is at most the lip above the
                // terrain, the shoulders' outer rows under it.
                assert!(q[1] <= top + 1e-4, "{q:?}");
                if kind == Kind::Shoulder as u8 && (q[2] - 50.0).abs() > 3.05 {
                    assert!(q[1] < terrain, "outer shoulder under the ground: {q:?}");
                }
            }
        }
        // The shoulder: 0.4 m + 1.5 × the drop.
        let sh = shoulder(Vec3::new(0.0, top, 53.0), Vec3::Z, &g, false);
        assert!((sh.width - (SHOULDER_MIN_M + SHOULDER_SLOPE * 0.08)).abs() < 1e-4);
        assert!((sh.outer_y - (terrain - SHOULDER_SINK_M)).abs() < 1e-4);
        assert!(!sh.wall && sh.bottom_y < sh.outer_y);
        // The tallest vertical face above the terrain is the lip (3 cm):
        // the old 2 m edge face stood 8 cm above the plain along the road.
        let above: f32 = m
            .positions
            .iter()
            .zip(&m.uvs_b)
            .filter(|(_, b)| decode_b(b[0]).1 == Kind::Wall as u8)
            .map(|(q, _)| q[1] - terrain)
            .fold(f32::MIN, f32::max);
        assert!(above <= 0.08 + 1e-4, "{above}");
        let walls_above = m
            .positions
            .iter()
            .zip(&m.uvs_b)
            .filter(|(q, b)| {
                decode_b(b[0]).1 == Kind::Wall as u8 && q[1] > top - SHOULDER_STEP_M + 1e-4
            })
            .count();
        // Only the lip's top row (11 points × 2 sides) is above the lip.
        assert_eq!(walls_above, 22);
    }

    #[test]
    fn deep_drops_get_walls_and_rising_ground_a_flat_verge() {
        // The ground 5 m below: a retaining wall down to below it.
        let sh = shoulder(Vec3::new(0.0, 10.0, 0.0), Vec3::X, &flat(5.0), false);
        assert!(sh.wall);
        assert!((sh.bottom_y - (5.0 - SKIRT_M)).abs() < 1e-4);
        // A cutting (ground above the road): the narrowest verge at the lip.
        let cut = shoulder(Vec3::new(0.0, 10.0, 0.0), Vec3::X, &flat(11.0), false);
        assert_eq!(cut.width, SHOULDER_MIN_M);
        assert!((cut.outer_y - (10.0 - SHOULDER_STEP_M)).abs() < 1e-5);
        // No terrain: a plain skirt.
        let none = shoulder(Vec3::new(0.0, 10.0, 0.0), Vec3::X, &|_, _| None, false);
        assert!((none.bottom_y - (10.0 - EDGE_DEPTH_M)).abs() < 1e-5);
        // In a junction zone the verge stays narrow.
        let j = shoulder(Vec3::new(0.0, 10.0, 0.0), Vec3::X, &flat(9.0), true);
        assert_eq!(j.width, SHOULDER_MIN_M);
    }

    #[test]
    fn kerbs_and_sidewalks_stand_beside_the_road() {
        let dress = RoadDress {
            sidewalk_m: [2.0, 0.0],
            ..RoadDress::of_class(RoadClass::Residential, 6.0)
        };
        let g = flat(0.0);
        let m = meshes(&[strip(RoadClass::Residential, 6.0, 0.05, dress)], &g).roads;
        let top = 0.05 + crate::ROAD_SURFACE_LIFT_M;
        let left = 50.0 - 3.0; // −right of +x is −z
        let sidewalk: Vec<&[f32; 3]> = m
            .positions
            .iter()
            .zip(&m.uvs_b)
            .filter(|(_, b)| decode_b(b[0]).1 == Kind::Sidewalk as u8)
            .map(|(q, _)| q)
            .collect();
        assert_eq!(sidewalk.len(), 22);
        // On the left (z < 47), 12 cm above the road, 2 m wide, falling 2 %
        // towards the road.
        for q in &sidewalk {
            assert!(q[2] <= left + 1e-4 && q[2] >= left - 2.0 - 1e-4, "{q:?}");
            assert!(
                q[1] >= top + 0.12 - 1e-4 && q[1] <= top + 0.12 + 0.04 + 1e-4,
                "{q:?}"
            );
        }
        // The kerb face faces the road (+z on the left side).
        let kerb: Vec<usize> = (0..m.positions.len())
            .filter(|&i| decode_b(m.uvs_b[i][0]).1 == Kind::Kerb as u8)
            .collect();
        assert_eq!(kerb.len(), 22);
        assert!(kerb.iter().all(|&i| m.normals[i][2] > 0.99));
        // The riding surface itself is unchanged: the carriageway's
        // vertices stay at the surface height, within the road width.
        for (q, b) in m.positions.iter().zip(&m.uvs_b) {
            if decode_b(b[0]).1 == Kind::Surface as u8 {
                assert!((q[1] - top).abs() < 1e-5 && (q[2] - 50.0).abs() <= 3.0 + 1e-4);
            }
        }
        // No sidewalk on the right: the shoulder starts at the road edge.
        assert!(m.positions.iter().all(|q| q[1] <= top + 0.2));
    }

    #[test]
    fn surface_vertices_carry_the_marking_code_and_the_arc_length() {
        let dress = RoadDress {
            start_m: 500.0,
            ..RoadDress::of_class(RoadClass::Primary, 7.5)
        };
        let m = meshes(
            &[strip(RoadClass::Primary, 7.5, 0.0, dress.clone())],
            &flat(-1.0),
        )
        .roads;
        let code = MarkCode::of(&dress, RoadClass::Primary, RoadSurface::Asphalt);
        assert_eq!(code.centre, CentreLine::Dashed);
        let surf: Vec<usize> = (0..m.positions.len())
            .filter(|&i| decode_b(m.uvs_b[i][0]).1 == Kind::Surface as u8)
            .collect();
        assert_eq!(surf.len(), 22);
        for &i in &surf {
            assert_eq!(m.uvs_b[i][1], code.lines_code() as f32);
            assert_eq!(m.colors[i][3], 3.75, "half width in the alpha");
        }
        assert_eq!(m.uvs[surf[0]][1], 500.0);
        assert!((m.uvs[surf[21]][1] - 600.0).abs() < 1e-3);
        // Codes survive the float round trip.
        let (layer, kind, extra) = decode_b(code_b(6, Kind::Crossing, 21));
        assert_eq!((layer, kind, extra), (6, Kind::Crossing as u8, 21));
    }

    #[test]
    fn crossings_become_overlays_over_their_painted_length() {
        let st = style();
        let dress = RoadDress {
            crossings: vec![Crossing {
                along_m: 40.0,
                kind: CrossingKind::Zebra,
                half: Half::Both,
            }],
            ..RoadDress::of_class(RoadClass::Residential, 6.0)
        };
        let m = meshes(
            &[strip(RoadClass::Residential, 6.0, 0.0, dress)],
            &flat(-1.0),
        )
        .overlays;
        assert!(!m.indices.is_empty());
        let xs: Vec<f32> = m.positions.iter().map(|q| q[0]).collect();
        let (lo, hi) = (
            xs.iter().copied().fold(f32::MAX, f32::min),
            xs.iter().copied().fold(f32::MIN, f32::max),
        );
        let len = st.zebra_length_m;
        assert!(
            (lo - (40.0 - len / 2.0 - OVERLAY_MARGIN_M)).abs() < 1e-3,
            "{lo}"
        );
        assert!(
            (hi - (40.0 + len / 2.0 + OVERLAY_MARGIN_M)).abs() < 1e-3,
            "{hi}"
        );
        // Just above the road surface, across its full width.
        assert!(
            m.positions
                .iter()
                .all(|q| (q[1] - (crate::ROAD_SURFACE_LIFT_M + OVERLAY_LIFT_M)).abs() < 1e-5)
        );
        assert!(
            m.uvs_b
                .iter()
                .all(|b| decode_b(b[0]).1 == Kind::Crossing as u8 && b[1] == 0.0)
        );
        // Not on cobbles.
        let mut cobbles = strip(RoadClass::Residential, 6.0, 0.0, RoadDress::default());
        cobbles.surface = RoadSurface::Paving;
        cobbles.dress.crossings = vec![Crossing {
            along_m: 40.0,
            kind: CrossingKind::Zebra,
            half: Half::Both,
        }];
        assert!(meshes(&[cobbles], &flat(-1.0)).overlays.indices.is_empty());
    }
}
