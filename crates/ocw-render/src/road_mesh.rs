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
/// Along a strip its verge widens by at most this much per metre (J6,
/// §4.8.8 Rule; where the ground asks for more, the verge slopes steeper
/// for a few metres).
pub const VERGE_WIDEN: f32 = 0.3;
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
    shoulder_blend(top, out, ground, if narrow { 1.0 } else { 0.0 })
}

/// [`shoulder`] between its own width (`narrow` 0) and its narrowest (1).
pub fn shoulder_blend(
    top: Vec3,
    out: Vec3,
    ground: &dyn Fn(f32, f32) -> Option<f32>,
    narrow: f32,
) -> Shoulder {
    shoulder_sized(top, out, ground, narrow, None)
}

/// [`shoulder_blend`], or `width` wide where given (not a wall).
pub fn shoulder_sized(
    top: Vec3,
    out: Vec3,
    ground: &dyn Fn(f32, f32) -> Option<f32>,
    narrow: f32,
    width: Option<f32>,
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
    let width = width.unwrap_or(
        SHOULDER_MIN_M + SHOULDER_SLOPE * drop.max(0.0) * (1.0 - narrow.clamp(0.0, 1.0)),
    );
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
    road_meshes_rows(strips, &[], ctx)
}

/// [`road_meshes`] with the carriageway edges of some strips given (strips
/// shaped for a junction surface, `crate::junction_mesh`): those are drawn
/// along their rows and get no sidewalk corner of `road_corners` (the
/// junction surface's outline has its own).
pub fn road_meshes_rows(
    strips: &[RoadStrip],
    rows: &[Option<&Rows>],
    ctx: &RoadContext<'_>,
) -> RoadMeshes {
    road_meshes_bands(strips, rows, &[], ctx)
}

/// The kerb of a band drawn for a path along a road without a sidewalk
/// of its own (m; J7).
pub const BAND_KERB_M: f32 = 0.08;

/// [`road_meshes_rows`] with the bands of the paths along some strips
/// (J7, §4.8.8 Rule): per point how far beyond the strip's edges [left,
/// right] its sidewalk reaches, at least.
pub fn road_meshes_bands(
    strips: &[RoadStrip],
    rows: &[Option<&Rows>],
    bands: &[Option<&[[f32; 2]]>],
    ctx: &RoadContext<'_>,
) -> RoadMeshes {
    let mut out = RoadMeshes::default();
    let shaped = |i: usize| rows.get(i).copied().flatten().is_some();
    let mut corners = crate::road_corners::corners(strips);
    corners.retain(|c| c.ends.iter().all(|e| !shaped(e.0)));
    let trims = crate::road_corners::trims(&corners, strips.len());
    for (i, (s, trim)) in strips.iter().zip(&trims).enumerate() {
        let r = rows.get(i).copied().flatten();
        let band = bands
            .get(i)
            .copied()
            .flatten()
            .filter(|b| b.len() == s.points.len());
        if r.is_none() && trim.iter().flatten().any(Option::is_some) {
            let cut = crate::road_corners::with_cuts(s, trim);
            let band = band.filter(|b| b.len() == cut.points.len());
            strip_mesh(&mut out.roads, &cut, ctx, trim, None, band);
        } else {
            strip_mesh(&mut out.roads, s, ctx, trim, r, band);
        }
        crossing_overlays(&mut out.overlays, s, ctx, r);
    }
    for c in &corners {
        crate::road_corners::corner_mesh(&mut out.roads, c, strips, ctx);
    }
    out
}

/// The carriageway edges of a strip: per point [left, right] (drawn
/// heights, before the lift).
pub type Rows = [[[f32; 3]; 2]];

fn strip_mesh(
    m: &mut MeshArrays,
    s: &RoadStrip,
    ctx: &RoadContext<'_>,
    trim: &[[Option<f32>; 2]; 2],
    rows: Option<&Rows>,
    band: Option<&[[f32; 2]]>,
) {
    let lift = Vec3::Y * crate::ROAD_SURFACE_LIFT_M;
    let p: Vec<Vec3> = s.points.iter().map(|q| Vec3::from(*q) + lift).collect();
    let n = p.len();
    if n < 2 {
        return;
    }
    let half = s.width_m / 2.0;
    let layer = s.surface.code();
    let code = MarkCode::of(&s.dress, s.class, s.surface, &s.condition);
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
    // The carriageway's edges and the direction to the right.
    let edges: Vec<[Vec3; 2]> = match rows {
        Some(r) if r.len() == n => r
            .iter()
            .map(|e| [Vec3::from(e[0]) + lift, Vec3::from(e[1]) + lift])
            .collect(),
        _ => (0..n)
            .map(|i| {
                let d = direction(&p, i);
                let r = Vec3::new(-d.z, 0.0, d.x);
                [p[i] - r * half, p[i] + r * half]
            })
            .collect(),
    };
    let rights: Vec<Vec3> = edges
        .iter()
        .map(|e| (e[1] - e[0]).with_y(0.0).normalize_or_zero())
        .collect();
    // The carriageway.
    let base = m.positions.len() as u32;
    for i in 0..n {
        for (k, side) in [-half, half].into_iter().enumerate() {
            vertex(
                m,
                edges[i][k],
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
    let look = DressLook::of(s, ctx);
    for (si, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        // A path along the road (J7): its sidewalk reaches the path's far
        // edge there (not in a junction zone without a junction surface).
        let band_at = |i: usize| -> f32 {
            band.map_or(0.0, |b| {
                if s.dress.in_junction(local[i]) {
                    0.0
                } else {
                    b[i][si]
                }
            })
        };
        let banded = (0..n).any(|i| band_at(i) > 0.0);
        let sidewalk = s.dress.sidewalk_m[si] > 0.0 || banded;
        let edge: Vec<Vec3> = edges.iter().map(|e| e[si]).collect();
        // Out square to the edge line (J6, §4.8.8 Rule): a row past a
        // junction's mouth may lie askew to it where the road bends there,
        // and a verge out along such a row reached back over the corner's
        // verge (the V at Letzanaweg). The end rows keep the row's
        // direction (the next tile's strip, the junction surface's corner
        // verge go on from there).
        let out: Vec<Vec3> = (0..n)
            .map(|i| {
                let r = rights[i] * side;
                if i == 0 || i == n - 1 {
                    return r;
                }
                let d = (edge[i + 1] - edge[i - 1]).with_y(0.0).normalize_or_zero();
                let nrm = Vec3::new(-d.z, 0.0, d.x);
                let nrm = if nrm.dot(r) < 0.0 { -nrm } else { nrm };
                if nrm.length_squared() < 0.25 { r } else { nrm }
            })
            .collect();
        let decks: Vec<bool> = edge
            .iter()
            .map(|top| (ctx.deck)([top.x, top.y - crate::ROAD_SURFACE_LIFT_M, top.z]))
            .collect();
        let kerbs: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                if sidewalk && !decks[i] {
                    let (w, k) = if s.dress.sidewalk_m[si] > 0.0 {
                        crate::road_corners::kerb_trimmed(
                            &s.dress,
                            si,
                            local[i],
                            local[n - 1],
                            &trim[si],
                        )
                    } else {
                        (0.0, 0.0)
                    };
                    let b = band_at(i);
                    if b > w {
                        let k = if s.dress.kerb_m > 0.0 {
                            s.dress.kerb_m
                        } else {
                            BAND_KERB_M
                        };
                        (b, k)
                    } else {
                        (w, k)
                    }
                } else {
                    (0.0, 0.0)
                }
            })
            .collect();
        // Narrow in junction zones (without a junction surface); at a
        // junction surface's mouth its own width, as the corner's verge
        // has there (§4.8.8 Rule, J5).
        let narrow: Vec<f32> = local
            .iter()
            .map(|&l| if s.dress.in_junction(l) { 1.0 } else { 0.0 })
            .collect();
        side_dress(
            m,
            &SideDress {
                side,
                half,
                sidewalk,
                edge: &edge,
                out: &out,
                along: &along,
                kerbs: &kerbs,
                decks: &decks,
                narrow: &narrow,
                corner: false,
            },
            &look,
            ctx,
        );
    }
}

/// The materials of a road's edge parts.
pub(crate) struct DressLook {
    pub shoulder_layer: u8,
    pub shoulder_colour: [f32; 3],
    pub wall_colour: [f32; 3],
    pub sidewalk_layer: u8,
    pub sidewalk_colour: [f32; 3],
    pub kerb_colour: [f32; 3],
}

impl DressLook {
    pub(crate) fn of(s: &RoadStrip, ctx: &RoadContext<'_>) -> Self {
        let sidewalk_layer = if s.dress.sidewalk_asphalt {
            RoadSurface::Asphalt.code()
        } else {
            SIDEWALK_LAYER
        };
        Self {
            shoulder_layer: shoulder_layer(s.surface),
            shoulder_colour: ctx.colour(shoulder_layer(s.surface), lin(0.55, 0.5, 0.42)),
            wall_colour: ctx.colour(KERB_LAYER, EDGE_COLOUR),
            sidewalk_layer,
            sidewalk_colour: ctx.colour(sidewalk_layer, SIDEWALK_COLOUR),
            kerb_colour: ctx.colour(KERB_LAYER, KERB_COLOUR),
        }
    }
}

/// One side of a road edge, as polylines of `n` points.
pub(crate) struct SideDress<'a> {
    /// −1: a strip's left side, +1 its right (winding and the sign of u).
    pub side: f32,
    pub half: f32,
    /// Draw a kerb and sidewalk (where `kerbs` gives them a width).
    pub sidewalk: bool,
    /// The carriageway's edge (drawn) and the outward direction.
    pub edge: &'a [Vec3],
    pub out: &'a [Vec3],
    pub along: &'a [f32],
    /// Sidewalk width and kerb height per point.
    pub kerbs: &'a [(f32, f32)],
    pub decks: &'a [bool],
    /// How far to keep the shoulder at its narrowest (1: junction zones;
    /// 0: its own width).
    pub narrow: &'a [f32],
    /// A junction surface's corner (J5, J6): the verge out to its own
    /// smooth outer edge ([`corner_outer`]; its ends as they are, where the
    /// arms' verges go on).
    pub corner: bool,
}

/// The kerb, sidewalk, lip, shoulder and skirt (or retaining wall) beside a
/// carriageway edge.
pub(crate) fn side_dress(
    m: &mut MeshArrays,
    d: &SideDress<'_>,
    look: &DressLook,
    ctx: &RoadContext<'_>,
) {
    let n = d.edge.len();
    if n < 2 {
        return;
    }
    let (side, half) = (d.side, d.half);
    // Per point: the edge top beyond the sidewalk (where the shoulder
    // starts).
    let tops: Vec<Vec3> = (0..n)
        .map(|i| {
            let (w, k) = d.kerbs[i];
            d.edge[i] + d.out[i] * w + Vec3::Y * (k + w * SIDEWALK_FALL)
        })
        .collect();
    if d.sidewalk {
        // Kerb face (towards the road), then the sidewalk surface; the
        // shader paints the kerb stones' top on its first centimetres.
        let kb = m.positions.len() as u32;
        for i in 0..n {
            let k = d.kerbs[i].1;
            for (y, u) in [(0.0, side * half), (k, side * (half - k))] {
                vertex(
                    m,
                    d.edge[i] + Vec3::Y * y,
                    -d.out[i],
                    look.kerb_colour,
                    half,
                    [u, d.along[i]],
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
        for (i, top) in tops.iter().enumerate() {
            let (w, k) = d.kerbs[i];
            let edge = d.edge[i] + Vec3::Y * k;
            for (q, u) in [(edge, side * half), (*top, side * (half + w))] {
                vertex(
                    m,
                    q,
                    Vec3::Y,
                    look.sidewalk_colour,
                    half,
                    [u, d.along[i]],
                    [code_b(look.sidewalk_layer, Kind::Sidewalk, 0), 0.0],
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
    let mut shoulders: Vec<Shoulder> = (0..n)
        .map(|i| {
            let top = tops[i];
            if d.decks[i] {
                Shoulder {
                    width: 0.0,
                    outer_y: top.y,
                    bottom_y: top.y,
                    wall: false,
                }
            } else {
                shoulder_blend(top, d.out[i], ctx.ground, d.narrow[i])
            }
        })
        .collect();
    // Along a strip the verge widens by at most VERGE_WIDEN per metre from
    // a narrower point (J6, §4.8.8 Rule): where the ground drops away just
    // past a junction's mouth the verge no longer jumps from the corner's
    // width to several metres within a row (a V of grass between the two
    // outer edges at Letzanaweg). Its end points stay (a junction's corner
    // verge and the next tile's strip go on from them); walls and decks
    // keep theirs.
    if !d.corner && n >= 3 {
        let mut w: Vec<f32> = shoulders.iter().map(|s| s.width).collect();
        let free_at: Vec<bool> = (0..n).map(|i| !d.decks[i] && !shoulders[i].wall).collect();
        let free = |i: usize| free_at[i];
        for i in 1..n - 1 {
            if free(i) && free(i - 1) {
                let l = (tops[i] - tops[i - 1]).with_y(0.0).length();
                w[i] = w[i].min(w[i - 1] + VERGE_WIDEN * l);
            }
        }
        for i in (1..n - 1).rev() {
            if free(i) && free(i + 1) {
                let l = (tops[i + 1] - tops[i]).with_y(0.0).length();
                w[i] = w[i].min(w[i + 1] + VERGE_WIDEN * l);
            }
        }
        for i in 1..n - 1 {
            if free(i) && w[i] < shoulders[i].width - 1e-4 {
                shoulders[i] = shoulder_sized(tops[i], d.out[i], ctx.ground, 0.0, Some(w[i]));
            }
        }
    }
    // The outward direction per point: a corner's verge (J6) goes out to
    // its own smooth outer edge, not square to the carriageway's edge.
    let mut outs: Vec<Vec3> = d.out.to_vec();
    // A strip's too (J6): the outer edge on the inner side of a tight
    // bend – a strip turning right past a junction's mouth – folds or
    // zigzags otherwise (a V of grass at Letzanaweg). Not on a bridge.
    if n >= 3 && (d.corner || !d.decks.iter().any(|&x| x)) {
        let outer = corner_outer(&tops, d.out, &shoulders, d.corner);
        for i in 1..n - 1 {
            if d.decks[i] || shoulders[i].wall {
                continue;
            }
            let v = (outer[i] - Vec2::new(tops[i].x, tops[i].z)).extend(0.0);
            let v = Vec3::new(v.x, 0.0, v.y);
            let w = v.length();
            if w < 1e-3 {
                continue;
            }
            outs[i] = v / w;
            shoulders[i] = shoulder_sized(tops[i], outs[i], ctx.ground, 0.0, Some(w));
        }
    }
    let d = &SideDress { out: &outs, ..*d };
    let e0 = side * half;
    let lip_of = |i: usize| {
        if d.decks[i] {
            tops[i].y
        } else {
            tops[i].y - SHOULDER_STEP_M
        }
    };
    // Row 0: top; row 1: lip; row 2: shoulder outer; row 3: skirt bottom.
    for row in 0..4 {
        for i in 0..n {
            let out = d.out[i];
            let top = tops[i];
            let sh = shoulders[i];
            let lip = lip_of(i);
            let e = e0 + side * d.kerbs[i].0;
            let outer = top + out * sh.width;
            let (q, u, kind, layer, colour, normal) = match row {
                0 => (top, e, Kind::Wall, KERB_LAYER, look.wall_colour, out),
                1 => (
                    top.with_y(lip),
                    e + side * 0.03,
                    Kind::Wall,
                    KERB_LAYER,
                    look.wall_colour,
                    out,
                ),
                2 => {
                    let n = (out * (lip - sh.outer_y) + Vec3::Y * sh.width.max(1e-3)).normalize();
                    if sh.wall {
                        (
                            outer.with_y(sh.outer_y),
                            e + side * sh.width,
                            Kind::Wall,
                            KERB_LAYER,
                            look.wall_colour,
                            n,
                        )
                    } else {
                        (
                            outer.with_y(sh.outer_y),
                            e + side * sh.width,
                            Kind::Shoulder,
                            look.shoulder_layer,
                            look.shoulder_colour,
                            n,
                        )
                    }
                }
                _ => (
                    outer.with_y(sh.bottom_y),
                    e + side * (sh.width + sh.outer_y - sh.bottom_y),
                    Kind::Wall,
                    KERB_LAYER,
                    look.wall_colour,
                    out,
                ),
            };
            vertex(
                m,
                q,
                normal,
                colour,
                half,
                [u, d.along[i]],
                [code_b(layer, kind, 0), 0.0],
            );
        }
    }
    // The lip faces out; the shoulder up; the skirt both ways (seen from
    // below a hairpin's upper leg too). Inner-edge vertices of the shoulder
    // are the lip's bottom row with the wall kind: the shoulder gets its
    // own copy for its material.
    let nn = n as u32;
    let (r0, r1, r2, r3) = (rows, rows + nn, rows + 2 * nn, rows + 3 * nn);
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
        let lip = lip_of(i);
        let out = d.out[i];
        let nrm = (out * (lip - sh.outer_y) + Vec3::Y * sh.width.max(1e-3)).normalize();
        let e = e0 + side * (d.kerbs[i].0 + 0.03);
        let (layer, kind, colour) = if sh.wall {
            (KERB_LAYER, Kind::Wall, look.wall_colour)
        } else {
            (look.shoulder_layer, Kind::Shoulder, look.shoulder_colour)
        };
        vertex(
            m,
            top.with_y(lip),
            nrm,
            colour,
            half,
            [e, d.along[i]],
            [code_b(layer, kind, 0), 0.0],
        );
    }
    face(m, si_row, r2, false);
    face(m, r2, r3, true);
}

/// The outer edge of a junction corner's verge bends at most as a curve
/// of this radius would (m; §4.8.8 Rule, J6), measured over pieces of
/// [`CORNER_VERGE_PIECE_M`], where the corner leaves room: round a corner
/// of a smaller radius it does not follow the carriageway's edge at the
/// verge's width (a fold, or the V of J5 where the width was cut to fit)
/// but runs as its own curve.
pub const CORNER_VERGE_RADIUS_M: f32 = 1.5;
/// Bends of a corner verge's outer edge are measured over pieces this long
/// (m), as the side lint measures the carriageway's.
pub const CORNER_VERGE_PIECE_M: f32 = 0.3;

/// A cubic Bézier from `a` leaving along `ua` to `b` arriving along `ub`
/// with handles `ha`, `hb`, at `t`.
fn bezier2(a: Vec2, ua: Vec2, b: Vec2, ub: Vec2, ha: f32, hb: f32, t: f32) -> Vec2 {
    let (c1, c2) = (a + ua * ha, b - ub * hb);
    let s = 1.0 - t;
    a * (s * s * s) + c1 * (3.0 * s * s * t) + c2 * (3.0 * s * t * t) + b * (t * t * t)
}

/// The point `s` along polyline `p` (lengths `cum`), straight on along
/// `ends` (the direction arriving at its first point, leaving its last)
/// past its ends.
fn along2(p: &[Vec2], cum: &[f32], ends: [Vec2; 2], s: f32) -> Vec2 {
    let n = p.len();
    let total = cum[n - 1];
    if s <= 0.0 {
        return p[0] + ends[0] * s;
    }
    if s >= total {
        return p[n - 1] + ends[1] * (s - total);
    }
    let k = cum.partition_point(|&c| c < s).clamp(1, n - 1);
    let l = (cum[k] - cum[k - 1]).max(1e-6);
    p[k - 1].lerp(p[k], (s - cum[k - 1]) / l)
}

fn lengths2(p: &[Vec2]) -> Vec<f32> {
    let mut cum = vec![0.0f32; p.len()];
    for i in 1..p.len() {
        cum[i] = cum[i - 1] + p[i].distance(p[i - 1]);
    }
    cum
}

/// The bend of polyline `p` at arc length `s` over pieces of
/// [`CORNER_VERGE_PIECE_M`] each way (radians).
fn bend2(p: &[Vec2], cum: &[f32], ends: [Vec2; 2], s: f32) -> f32 {
    let h = CORNER_VERGE_PIECE_M;
    let (p0, p1, p2) = (
        along2(p, cum, ends, s - h),
        along2(p, cum, ends, s),
        along2(p, cum, ends, s + h),
    );
    let (a, b) = (p1 - p0, p2 - p1);
    if a.length() < 1e-5 || b.length() < 1e-5 {
        return std::f32::consts::PI;
    }
    a.angle_to(b).abs()
}

/// The worst bend along polyline `p` (every 10 cm).
fn worst_bend2(p: &[Vec2], ends: [Vec2; 2]) -> f32 {
    let cum = lengths2(p);
    let total = cum[cum.len() - 1];
    let mut worst = 0.0f32;
    let mut s = 0.0;
    while s <= total + 1e-4 {
        worst = worst.max(bend2(p, &cum, ends, s));
        s += 0.1;
    }
    worst
}

/// The outer edge of a junction corner's verge (J6), a point per point of
/// the corner's edge (`tops`): each point's own width (by the ground's
/// drop) smoothed along the corner, the offset line's loops cut out where
/// the corner turns tighter than the verge is wide, and the stretches that
/// still bend more than [`CORNER_VERGE_RADIUS_M`] allows replaced by a
/// cubic curve tangent to the line on both sides (reaching farther along
/// it until it bends little enough, at most to the corner's ends) – its
/// two ends kept where the arms' verges end and leaving them along the
/// corner's first and last pieces, so it runs on from the arms' verges'
/// outer edges without a step or a V; never nearer the edge than the
/// narrowest verge.
fn corner_outer(tops: &[Vec3], out: &[Vec3], sh: &[Shoulder], corner: bool) -> Vec<Vec2> {
    let n = tops.len();
    let xz = |v: Vec3| Vec2::new(v.x, v.z);
    let inner: Vec<Vec2> = tops.iter().map(|t| xz(*t)).collect();
    let cum = lengths2(&inner);
    // The widths smoothed along the corner, the ends kept.
    let mut w: Vec<f32> = (0..n)
        .map(|i| {
            if sh[i].wall {
                SHOULDER_MIN_M
            } else {
                sh[i].width
            }
        })
        .collect();
    for _ in 0..4 {
        let prev = w.clone();
        for i in 1..n - 1 {
            let (l0, l1) = (cum[i] - cum[i - 1], cum[i + 1] - cum[i]);
            w[i] =
                0.5 * prev[i] + 0.5 * (prev[i - 1] * l1 + prev[i + 1] * l0) / (l0 + l1).max(1e-6);
        }
    }
    let mut q: Vec<Vec2> = (0..n)
        .map(|i| inner[i] + Vec2::new(out[i].x, out[i].z) * w[i])
        .collect();
    // Loops cut out: where two pieces cross, the points between go to the
    // crossing.
    let cross = |a: Vec2, b: Vec2, c: Vec2, d: Vec2| -> Option<Vec2> {
        let (r, s) = (b - a, d - c);
        let den = r.perp_dot(s);
        if den.abs() < 1e-9 {
            return None;
        }
        let t = (c - a).perp_dot(s) / den;
        let u = (c - a).perp_dot(r) / den;
        ((1e-4..=1.0 - 1e-4).contains(&t) && (1e-4..=1.0 - 1e-4).contains(&u)).then(|| a + r * t)
    };
    'cut: loop {
        for i in 0..n - 1 {
            // (Loops of an offset line are local: within 40 points.)
            for j in (i + 2)..(n - 1).min(i + 40) {
                if let Some(x) = cross(q[i], q[i + 1], q[j], q[j + 1]) {
                    for p in &mut q[i + 1..=j] {
                        *p = x;
                    }
                    continue 'cut;
                }
            }
        }
        break;
    }
    // Along the arms past the ends (the arms' verges go on so).
    let t_in = (inner[0] - inner[1]).normalize_or_zero();
    let t_out = (inner[n - 1] - inner[n - 2]).normalize_or_zero();
    let ends = [-t_in, t_out];
    // The two arms' verges cross beyond the mouths (a verge wider than a
    // short corner leaves room for): they cover the corner's outside, the
    // corner's verge runs straight from the one's outer end to the
    // other's, under them.
    if corner {
        let (a, b) = (q[0], q[n - 1]);
        let den = t_in.perp_dot(t_out);
        if den.abs() > 1e-6 {
            let s = (b - a).perp_dot(t_out) / den;
            let u = (b - a).perp_dot(t_in) / den;
            if s > 0.0 && u > 0.0 {
                let total = cum[n - 1].max(1e-6);
                return (0..n).map(|i| a.lerp(b, cum[i] / total)).collect();
            }
        }
    }
    let limit = 2.0 * (CORNER_VERGE_PIECE_M / (2.0 * CORNER_VERGE_RADIUS_M)).asin();
    let qcum = lengths2(&q);
    let tight: Vec<usize> = (0..n)
        .filter(|&k| bend2(&q, &qcum, ends, qcum[k]) > limit * 1.02)
        .collect();
    let mut res = q.clone();
    if tight.is_empty() {
        return res;
    }
    // Neighbouring tight points (at most two apart) make one stretch.
    let mut regions: Vec<[usize; 2]> = Vec::new();
    for &k in &tight {
        match regions.last_mut() {
            Some(r) if k <= r[1] + 2 => r[1] = k,
            _ => regions.push([k, k]),
        }
    }
    // The point `metres` (of the corner's edge) before / after `k`.
    let reach = |k: usize, fwd: bool, metres: f32| -> usize {
        let mut j = k;
        loop {
            let nj = if fwd { j + 1 } else { j.wrapping_sub(1) };
            if nj >= n {
                return j;
            }
            j = nj;
            if (cum[j] - cum[k]).abs() >= metres {
                return j;
            }
        }
    };
    let dir_before = |k: usize| -> Vec2 {
        (0..k)
            .rev()
            .map(|j| q[k] - q[j])
            .find(|d| d.length() > 1e-5)
            .map_or(-t_in, |d| d.normalize())
    };
    let dir_after = |k: usize| -> Vec2 {
        (k + 1..n)
            .map(|j| q[j] - q[k])
            .find(|d| d.length() > 1e-5)
            .map_or(t_out, |d| d.normalize())
    };
    const HANDLES: [f32; 5] = [0.15, 0.3, 0.45, 0.65, 0.9];
    let mut done_to = 0usize;
    for [a0, b0] in regions {
        if b0 < done_to {
            continue;
        }
        // (worst bend, a, b, ua, ub, handles)
        let mut best: Option<(f32, usize, usize, Vec2, Vec2, [f32; 2])> = None;
        for grow in 0..30 {
            let m = CORNER_VERGE_PIECE_M + 0.75 * grow as f32;
            let (a, b) = (reach(a0, false, m).max(done_to), reach(b0, true, m));
            let (pa, pb) = (q[a], q[b]);
            let ua = if a > 0 { dir_before(a) } else { -t_in };
            let ub = if b < n - 1 { dir_after(b) } else { t_out };
            let chord = pa.distance(pb);
            if chord > 1e-3 {
                for ka in HANDLES {
                    for kb in HANDLES {
                        let h = [ka * chord, kb * chord];
                        let pts: Vec<Vec2> = (0..=24)
                            .map(|i| bezier2(pa, ua, pb, ub, h[0], h[1], i as f32 / 24.0))
                            .collect();
                        let wb = worst_bend2(&pts, [ua, ub]);
                        if best.is_none_or(|x| wb < x.0 - 1e-6) {
                            best = Some((wb, a, b, ua, ub, h));
                        }
                    }
                }
            }
            if best.is_some_and(|x| x.0 <= limit * 1.02) || (a == 0 && b == n - 1) {
                break;
            }
        }
        let Some((_, a, b, ua, ub, h)) = best else {
            continue;
        };
        let l = (cum[b] - cum[a]).max(1e-6);
        for (k, r) in res.iter_mut().enumerate().take(b).skip(a + 1) {
            let f = (cum[k] - cum[a]) / l;
            *r = bezier2(q[a], ua, q[b], ub, h[0], h[1], f);
        }
        done_to = b;
    }
    // Never nearer the edge than the narrowest verge.
    for i in 1..n - 1 {
        let o = Vec2::new(out[i].x, out[i].z);
        let v = res[i] - inner[i];
        let d = v.dot(o);
        if d < SHOULDER_MIN_M {
            res[i] += o * (SHOULDER_MIN_M - d);
        }
    }
    res
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
fn crossing_overlays(
    m: &mut MeshArrays,
    s: &RoadStrip,
    ctx: &RoadContext<'_>,
    rows: Option<&Rows>,
) {
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
        let lift = Vec3::Y * (crate::ROAD_SURFACE_LIFT_M + OVERLAY_LIFT_M);
        for j in 0..=steps {
            let t = -OVERLAY_MARGIN_M + (len + 2.0 * OVERLAY_MARGIN_M) * j as f32 / steps as f32;
            let (q, d) = at_along(&p, &cum, start + t);
            let right = Vec3::new(-d.z, 0.0, d.x);
            // On a shaped strip the overlay lies on its rows (not flat
            // across).
            let edges = rows.filter(|r| r.len() == p.len()).map(|r| {
                let a = start + t;
                let k = cum.partition_point(|&c| c <= a).clamp(1, p.len() - 1);
                let seg = (cum[k] - cum[k - 1]).max(1e-6);
                let f = ((a - cum[k - 1]) / seg).clamp(0.0, 1.0);
                let e =
                    |side: usize| Vec3::from(r[k - 1][side]).lerp(Vec3::from(r[k][side]), f) + lift;
                [e(0), e(1)]
            });
            for (k, side) in [-half, half].into_iter().enumerate() {
                vertex(
                    m,
                    edges.map_or(q + right * side, |e| e[k]),
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
            condition: Default::default(),
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
        let code = MarkCode::of(
            &dress,
            RoadClass::Primary,
            RoadSurface::Asphalt,
            &Default::default(),
        );
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
        let (_, _, extra) = decode_b(code_b(1, Kind::Surface, 255));
        assert_eq!(extra, 255, "the wear grade's bits survive too");
        // The wear grade rides along in the extra bits.
        let (_, _, extra) = decode_b(m.uvs_b[surf[0]][0]);
        assert_eq!(MarkCode::decode(0, extra).wear, code.wear);
        assert!(code.wear > 0);
    }

    /// Road damage is drawn only (spec §4.6, *Road look*): a worn minor
    /// road and a kept-up main road on the same line have the same
    /// geometry – the ridden surface does not change with the wear grade.
    #[test]
    fn the_wear_grade_changes_no_geometry() {
        let dress = RoadDress::of_class(RoadClass::Primary, 6.0);
        let main = strip(RoadClass::Primary, 6.0, 0.0, dress.clone());
        let mut worn = strip(RoadClass::Primary, 6.0, 0.0, dress.clone());
        worn.class = RoadClass::Unclassified;
        worn.surface = RoadSurface::WornAsphalt;
        worn.condition.smoothness = ocw_world::road_condition::Smoothness::VeryBad;
        let (a, b) = (
            meshes(std::slice::from_ref(&main), &flat(-0.1)).roads,
            meshes(std::slice::from_ref(&worn), &flat(-0.1)).roads,
        );
        let ga = MarkCode::of(&main.dress, main.class, main.surface, &main.condition).wear;
        let gb = MarkCode::of(&worn.dress, worn.class, worn.surface, &worn.condition).wear;
        assert!(gb > ga + 2, "{ga} {gb}");
        assert_eq!(a.positions, b.positions);
        assert_eq!(a.normals, b.normals);
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.uvs, b.uvs);
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
