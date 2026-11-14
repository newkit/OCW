//! Rounded sidewalk corners at junctions (spec §14 Phase 8 T5; open since
//! T5: the sidewalks ended in a wedge where two of them met). Where two
//! roads of a tile meet at a junction 20°–160° apart, both with a sidewalk
//! on the sides that face each other, the kerb runs round the corner on a
//! circular arc tangent to both kerb lines (the radius from the junction
//! zones, at least the sidewalks' width and 1 m, at most 10 m), the
//! sidewalk follows it as an annular sector, and the corner inside the arc
//! is road surface. The two strips' sidewalks end square at the arc's
//! tangent points ([`Trims`]) instead of tapering into the junction zone.
//! Towns split their streets into short edges (crossings, tag changes): an
//! arm shorter than its tangent point runs on into the strip that continues
//! it (end to end, not at a junction), whose sidewalk is trimmed too.
//!
//! [`corners`] is the pure geometry (tests), [`corner_mesh`] draws one in
//! the road meshes' vertex format (`road_mesh`, `road.wesl`).

use bevy::math::{Vec2, Vec3};
use ocw_world::RoadDress;
use ocw_world::package::RoadStrip;
use ocw_world::road_dress::{KERB_LAYER, KERB_RAMP_M, SIDEWALK_FALL, SIDEWALK_LAYER};

use crate::package::MeshArrays;
use crate::road_mesh::{
    EDGE_COLOUR, KERB_COLOUR, Kind, RoadContext, SIDEWALK_COLOUR, SKIRT_M, code_b,
};

/// Arms closer than this (m) end at the same junction.
const SAME_JUNCTION_M: f32 = 0.5;
/// Corners between arms this far apart (rad) only: 20° … 160°.
const ANGLE_RANGE: (f32, f32) = (0.35, 2.8);
/// Kerb radius limits (m).
const RADIUS_M: (f32, f32) = (1.0, 10.0);
/// Arc segments: at most this angle each (rad, 10°).
const ARC_STEP: f32 = 0.1745;

/// One end of a strip at a junction.
#[derive(Debug, Clone, Copy)]
struct Arm {
    strip: usize,
    /// 0: the strip's start, 1: its end.
    end: usize,
    /// The junction point (the strip's end point, horizontal).
    at: Vec2,
    /// Horizontal unit direction from the junction along the strip.
    dir: Vec2,
    half: f32,
    /// The strip's length (m, along its points).
    len: f32,
    /// The junction zone's length from this end (m).
    zone: f32,
}

impl Arm {
    /// The strip's side (0 left, 1 right) on the arm's right (`right`) or
    /// left: an arm at the strip's end looks against it.
    fn strip_side(&self, right: bool) -> usize {
        usize::from(right == (self.end == 0))
    }
}

/// A rounded corner.
#[derive(Debug, Clone, PartialEq)]
pub struct Corner {
    /// The sidewalks it replaces: (strip, end 0 / 1, strip side 0 / 1,
    /// where its sidewalk starts now, in m along the strip from that end;
    /// past the strip's length: none left on that side).
    pub ends: Vec<(usize, usize, usize, f32)>,
    /// Where the two kerb lines cross (x, z).
    pub p: Vec2,
    /// The arc's centre and radius (the kerb).
    pub centre: Vec2,
    pub radius: f32,
    /// The kerb from the first tangent point to the second (x, z), and per
    /// point the road surface's height (drawn), the kerb's height and the
    /// sidewalk's width.
    pub arc: Vec<Vec2>,
    pub y: Vec<f32>,
    pub kerb: Vec<f32>,
    pub width: Vec<f32>,
}

/// Where the strips' sidewalks start now: per strip, per side (0 left, 1
/// right), per end (0 start, 1 end), metres from that end.
pub type Trims = Vec<[[Option<f32>; 2]; 2]>;

fn horizontal(p: &[f32; 3]) -> Vec2 {
    Vec2::new(p[0], p[2])
}

/// Cumulative arc lengths of a strip's points (3D, as `road_mesh` counts).
fn arc_lengths(s: &RoadStrip) -> Vec<f32> {
    let mut out = vec![0.0f32; s.points.len()];
    for i in 1..s.points.len() {
        out[i] = out[i - 1] + (Vec3::from(s.points[i]) - Vec3::from(s.points[i - 1])).length();
    }
    out
}

/// The point at arc length `d` from the start (clamped), interpolated.
fn point_at(s: &RoadStrip, arc: &[f32], d: f32) -> Vec3 {
    let n = s.points.len();
    let d = d.clamp(0.0, arc[n - 1]);
    let i = arc.partition_point(|a| *a <= d).clamp(1, n - 1);
    let (a, b) = (Vec3::from(s.points[i - 1]), Vec3::from(s.points[i]));
    let seg = arc[i] - arc[i - 1];
    if seg <= 1e-6 {
        return a;
    }
    a.lerp(b, (d - arc[i - 1]) / seg)
}

/// The point `d` m along the strip from end `end`.
fn point_from(s: &RoadStrip, arc: &[f32], end: usize, d: f32) -> Vec3 {
    let len = arc[arc.len() - 1];
    point_at(s, arc, if end == 0 { d } else { len - d })
}

fn arms(strips: &[RoadStrip]) -> Vec<Arm> {
    let mut out = Vec::new();
    for (i, s) in strips.iter().enumerate() {
        if s.points.len() < 2 {
            continue;
        }
        let arc = arc_lengths(s);
        let len = arc[arc.len() - 1];
        let d = &s.dress;
        let mut push = |end: usize, zone: f32| {
            let at = point_from(s, &arc, end, 0.0);
            let ahead = point_from(s, &arc, end, (zone + KERB_RAMP_M).max(4.0).min(len));
            let dir =
                (horizontal(&ahead.to_array()) - horizontal(&at.to_array())).normalize_or_zero();
            if dir != Vec2::ZERO {
                out.push(Arm {
                    strip: i,
                    end,
                    at: horizontal(&at.to_array()),
                    dir,
                    half: s.width_m / 2.0,
                    len,
                    zone,
                });
            }
        };
        if d.junction_m[0] > 0.0 && d.start_m < 0.5 {
            push(0, d.junction_m[0] - d.start_m);
        }
        if d.junction_m[1] > 0.0 && d.edge_m > 0.0 && (d.start_m + len - d.edge_m).abs() < 0.5 {
            push(1, d.junction_m[1] - (d.edge_m - d.start_m - len));
        }
    }
    out
}

fn right(d: Vec2) -> Vec2 {
    Vec2::new(-d.y, d.x)
}

/// Strip ends that continue each other (end to end, not at a junction, the
/// same width, running on within 30°): (strip, end) → (strip, end).
type Links = std::collections::HashMap<(usize, usize), (usize, usize)>;

/// Strip ends by their position (0.1 m cells): (strip, end, point, outward
/// direction).
type EndCells = std::collections::HashMap<(i64, i64), Vec<(usize, usize, Vec2, Vec2)>>;

fn links(strips: &[RoadStrip]) -> Links {
    let mut ends = EndCells::default();
    for (i, s) in strips.iter().enumerate() {
        let n = s.points.len();
        if n < 2 {
            continue;
        }
        for end in 0..2 {
            if s.dress.junction_m[end] > 0.0 {
                continue;
            }
            let (k, k2) = if end == 0 { (0, 1) } else { (n - 1, n - 2) };
            let at = horizontal(&s.points[k]);
            // Outward from the strip at that end.
            let out = (at - horizontal(&s.points[k2])).normalize_or_zero();
            let key = ((at.x * 10.0).round() as i64, (at.y * 10.0).round() as i64);
            ends.entry(key).or_default().push((i, end, at, out));
        }
    }
    let mut out = Links::new();
    for group in ends.values() {
        for &(i, ei, a, da) in group {
            for &(j, ej, b, db) in group {
                if i != j
                    && a.distance(b) < 0.05
                    && da.dot(db) < -0.87
                    && (strips[i].width_m - strips[j].width_m).abs() < 0.01
                {
                    out.insert((i, ei), (j, ej));
                }
            }
        }
    }
    out
}

/// The strips along an arm from its junction: (strip, its end towards the
/// junction, the arm distance where it starts, its length); the arm's own
/// strip and the one continuing it, if any.
fn pieces(strips: &[RoadStrip], links: &Links, a: &Arm) -> Vec<(usize, usize, f32, f32)> {
    let mut out = vec![(a.strip, a.end, 0.0, a.len)];
    if let Some(&(j, ej)) = links.get(&(a.strip, 1 - a.end)) {
        let arc = arc_lengths(&strips[j]);
        out.push((j, ej, a.len, arc[arc.len() - 1]));
    }
    out
}

/// Where an arm's sidewalk on its right (`right`) or left starts at arm
/// distance `t`: the trims of its pieces, the point there and the
/// sidewalk's width and kerb (`None`: beyond the pieces, or no sidewalk).
#[allow(clippy::type_complexity)]
fn along_arm(
    strips: &[RoadStrip],
    pieces: &[(usize, usize, f32, f32)],
    right: bool,
    t: f32,
) -> Option<(Vec<(usize, usize, usize, f32)>, Vec3, f32, f32)> {
    let mut trims = Vec::new();
    for &(s, e, from, len) in pieces {
        let side = usize::from(right == (e == 0));
        if t > from + len - 1.0 {
            // Wholly before the tangent point: no sidewalk on that side.
            trims.push((s, e, side, len + 1.0));
            continue;
        }
        let st = &strips[s];
        let w = st.dress.sidewalk_m[side];
        if w <= 0.0 {
            return None;
        }
        let local = (t - from).max(0.0);
        trims.push((s, e, side, local));
        let arc = arc_lengths(st);
        return Some((trims, point_from(st, &arc, e, local), w, st.dress.kerb_m));
    }
    None
}

/// The rounded corners of a tile's strips (see the module docs).
pub fn corners(strips: &[RoadStrip]) -> Vec<Corner> {
    let all = arms(strips);
    let links = links(strips);
    let mut done = vec![false; all.len()];
    let mut out = Vec::new();
    for i in 0..all.len() {
        if done[i] {
            continue;
        }
        let mut group: Vec<Arm> = Vec::new();
        for j in i..all.len() {
            if !done[j] && all[j].at.distance(all[i].at) < SAME_JUNCTION_M {
                done[j] = true;
                group.push(all[j]);
            }
        }
        if group.len() < 2 {
            continue;
        }
        group.sort_by(|a, b| a.dir.y.atan2(a.dir.x).total_cmp(&b.dir.y.atan2(b.dir.x)));
        let n = group.len();
        for k in 0..n {
            let (a1, a2) = (group[k], group[(k + 1) % n]);
            if a1.strip == a2.strip {
                continue;
            }
            if let Some(c) = corner(strips, &links, &a1, &a2) {
                out.push(c);
            }
        }
    }
    out
}

/// The corner between arm `a1` and the next arm `a2` counter-clockwise
/// (seen from above with x right and z down: increasing atan2(z, x)) – the
/// block right of `a1` and left of `a2`.
fn corner(strips: &[RoadStrip], links: &Links, a1: &Arm, a2: &Arm) -> Option<Corner> {
    let (d1, d2) = (a1.dir, a2.dir);
    let theta = {
        let t = d2.y.atan2(d2.x) - d1.y.atan2(d1.x);
        t.rem_euclid(std::f32::consts::TAU)
    };
    if !(ANGLE_RANGE.0..=ANGLE_RANGE.1).contains(&theta) {
        return None;
    }
    // The facing sidewalks at the junction (the first pieces').
    let (s1, s2) = (a1.strip_side(true), a2.strip_side(false));
    let (w1, w2) = (
        strips[a1.strip].dress.sidewalk_m[s1],
        strips[a2.strip].dress.sidewalk_m[s2],
    );
    if w1 <= 0.0 || w2 <= 0.0 {
        return None;
    }
    let (n1, n2) = (right(d1), -right(d2));
    // Kerb lines J + d1 t + n1 h1 and J + d2 s + n2 h2 cross at P.
    let rhs = n2 * a2.half - n1 * a1.half + (a2.at - a1.at);
    let det = -d1.x * d2.y + d2.x * d1.y;
    if det.abs() < 1e-4 {
        return None;
    }
    let t = (rhs.x * -d2.y + d2.x * rhs.y) / det;
    let s = (d1.x * rhs.y - d1.y * rhs.x) / det;
    let k = 1.0 / (theta / 2.0).tan();
    // Tangent points in the middle of the kerb ramps, the radius at least
    // the sidewalk's width (the outer edge stays an arc).
    let fit = ((a1.zone + KERB_RAMP_M / 2.0 - t) / k).min((a2.zone + KERB_RAMP_M / 2.0 - s) / k);
    let mut radius = fit.max(w1.max(w2) + 0.5).clamp(RADIUS_M.0, RADIUS_M.1);
    let (p1, p2) = (pieces(strips, links, a1), pieces(strips, links, a2));
    // The sidewalk at the tangent point may be wider than at the junction
    // (on the strip that continues a short arm): widen the arc for it.
    let mut fitted = None;
    for _ in 0..3 {
        let (t1, t2) = (t + radius * k, s + radius * k);
        if t1 <= 0.0 || t2 <= 0.0 {
            return None;
        }
        let at1 = along_arm(strips, &p1, true, t1)?;
        let at2 = along_arm(strips, &p2, false, t2)?;
        let widest = at1.2.max(at2.2);
        if radius >= widest + 0.1 {
            fitted = Some((t1, t2, at1, at2));
            break;
        }
        let need = (widest + 0.5).min(RADIUS_M.1);
        if need <= radius {
            return None;
        }
        radius = need;
    }
    let (t1, t2, (trims1, q1, w1, k1), (trims2, q2, w2, k2)) = fitted?;
    let p = a1.at + d1 * t + n1 * a1.half;
    let bis = (d1 + d2).normalize();
    let centre = p + bis * radius / (theta / 2.0).sin();
    let ta = a1.at + d1 * t1 + n1 * a1.half;
    let tb = a2.at + d2 * t2 + n2 * a2.half;
    let (u0, u1) = ((ta - centre).to_angle(), (tb - centre).to_angle());
    let mut sweep = (u1 - u0).rem_euclid(std::f32::consts::TAU);
    if sweep > std::f32::consts::PI {
        sweep -= std::f32::consts::TAU;
    }
    let steps = ((sweep.abs() / ARC_STEP).ceil() as usize).max(2);
    let lift = crate::ROAD_SURFACE_LIFT_M;
    let (ya, yb) = (q1.y + lift, q2.y + lift);
    let mut c = Corner {
        ends: trims1.into_iter().chain(trims2).collect(),
        p,
        centre,
        radius,
        arc: Vec::with_capacity(steps + 1),
        y: Vec::with_capacity(steps + 1),
        kerb: Vec::with_capacity(steps + 1),
        width: Vec::with_capacity(steps + 1),
    };
    for j in 0..=steps {
        let u = j as f32 / steps as f32;
        c.arc
            .push(centre + Vec2::from_angle(u0 + sweep * u) * radius);
        c.y.push(ya + (yb - ya) * u);
        c.kerb.push(k1 + (k2 - k1) * u);
        c.width.push(w1 + (w2 - w1) * u);
    }
    // The tangent points exactly (no rounding drift at the strips' cuts).
    c.arc[0] = ta;
    c.arc[steps] = tb;
    Some(c)
}

/// The sidewalk starts of `corners` per strip ([`Trims`]).
pub fn trims(corners: &[Corner], strips: usize) -> Trims {
    let mut out: Trims = vec![[[None; 2]; 2]; strips];
    for c in corners {
        for &(strip, end, side, t) in &c.ends {
            let slot = &mut out[strip][side][end];
            *slot = Some(slot.map_or(t, |o: f32| o.max(t)));
        }
    }
    out
}

/// `s` with points inserted 2 cm before and at each sidewalk start of
/// `trim` (the sidewalk then ends square there, not over a segment).
pub fn with_cuts(s: &RoadStrip, trim: &[[Option<f32>; 2]; 2]) -> RoadStrip {
    let arc = arc_lengths(s);
    let len = arc[arc.len() - 1];
    let mut cuts: Vec<f32> = Vec::new();
    for side in trim {
        if let Some(t) = side[0] {
            cuts.extend([t - 0.02, t]);
        }
        if let Some(t) = side[1] {
            cuts.extend([len - t, len - t + 0.02]);
        }
    }
    cuts.retain(|d| *d > 0.01 && *d < len - 0.01);
    if cuts.is_empty() {
        return s.clone();
    }
    let mut all: Vec<(f32, [f32; 3])> = arc.iter().copied().zip(s.points.iter().copied()).collect();
    for d in cuts {
        if all.iter().all(|(a, _)| (a - d).abs() > 0.005) {
            all.push((d, point_at(s, &arc, d).to_array()));
        }
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    RoadStrip {
        points: all.into_iter().map(|(_, p)| p).collect(),
        ..s.clone()
    }
}

/// The sidewalk width and kerb height on `side` at `local` (m from the
/// strip's start; `len` its length) with the corners' `trim` of that side:
/// nothing before a start, full width after it (the corner continues it);
/// the junction-zone taper only at ends without a corner.
pub fn kerb_trimmed(
    dress: &RoadDress,
    side: usize,
    local: f32,
    len: f32,
    trim: &[Option<f32>; 2],
) -> (f32, f32) {
    if trim[0].is_some_and(|t| local < t - 1e-3) || trim[1].is_some_and(|t| len - local < t - 1e-3)
    {
        return (0.0, 0.0);
    }
    if trim[0].is_none() && trim[1].is_none() {
        return dress.kerb_at(side, local);
    }
    let mut d = dress.clone();
    for (e, t) in trim.iter().enumerate() {
        if t.is_some() {
            d.junction_m[e] = 0.0;
        }
    }
    d.kerb_at(side, local)
}

/// Pushes a triangle facing `up` (its winding turned to match).
fn tri(m: &mut MeshArrays, a: u32, b: u32, c: u32, up: Vec3) {
    let p = |i: u32| Vec3::from(m.positions[i as usize]);
    let n = (p(b) - p(a)).cross(p(c) - p(a));
    if n.dot(up) >= 0.0 {
        m.indices.extend([a, b, c]);
    } else {
        m.indices.extend([a, c, b]);
    }
}

#[allow(clippy::too_many_arguments)]
fn vertex(
    m: &mut MeshArrays,
    p: Vec3,
    n: Vec3,
    colour: [f32; 3],
    half: f32,
    uv: [f32; 2],
    b: f32,
) -> u32 {
    let i = m.positions.len() as u32;
    m.positions.push(p.to_array());
    m.normals.push(n.to_array());
    m.colors.push([colour[0], colour[1], colour[2], half]);
    m.uvs.push(uv);
    m.uvs_b.push([b, 0.0]);
    i
}

/// Draws corner `c` of `strips` into `m`: the road surface inside the arc
/// (a fan from the kerb lines' crossing), the kerb face, the sidewalk and
/// a skirt under its outer edge.
pub fn corner_mesh(m: &mut MeshArrays, c: &Corner, strips: &[RoadStrip], ctx: &RoadContext<'_>) {
    let first = &strips[c.ends[0].0];
    let half = first.width_m / 2.0;
    let layer = first.surface.code();
    let road_colour = ctx.colour(layer, {
        let r = crate::package::road_colour(first.class, first.surface);
        [r[0], r[1], r[2]]
    });
    let sidewalk_layer = if first.dress.sidewalk_asphalt {
        ocw_world::RoadSurface::Asphalt.code()
    } else {
        SIDEWALK_LAYER
    };
    let sidewalk_colour = ctx.colour(sidewalk_layer, SIDEWALK_COLOUR);
    let kerb_colour = ctx.colour(KERB_LAYER, KERB_COLOUR);
    let wall_colour = ctx.colour(KERB_LAYER, EDGE_COLOUR);
    let n = c.arc.len();
    let at = |j: usize, r: f32, y: f32| {
        let q = c.centre + (c.arc[j] - c.centre).normalize_or_zero() * r;
        Vec3::new(q.x, y, q.y)
    };
    let mut along = vec![0.0f32; n];
    for j in 1..n {
        along[j] = along[j - 1] + c.arc[j].distance(c.arc[j - 1]);
    }
    let out = |j: usize| {
        let d = (c.arc[j] - c.centre).normalize_or_zero();
        Vec3::new(d.x, 0.0, d.y)
    };
    // The road surface inside the arc.
    let mid = c.y[n / 2];
    let apex = vertex(
        m,
        Vec3::new(c.p.x, mid, c.p.y),
        Vec3::Y,
        road_colour,
        half,
        [0.0, 0.0],
        code_b(layer, Kind::Surface, 0),
    );
    let fan: Vec<u32> = (0..n)
        .map(|j| {
            vertex(
                m,
                at(j, c.radius, c.y[j]),
                Vec3::Y,
                road_colour,
                half,
                [half, along[j]],
                code_b(layer, Kind::Surface, 0),
            )
        })
        .collect();
    for j in 0..n - 1 {
        tri(m, apex, fan[j], fan[j + 1], Vec3::Y);
    }
    // The kerb face, towards the road.
    let kb = code_b(KERB_LAYER, Kind::Kerb, 0);
    let rows: Vec<(u32, u32)> = (0..n)
        .map(|j| {
            let b = vertex(
                m,
                at(j, c.radius, c.y[j]),
                out(j),
                kerb_colour,
                half,
                [half, along[j]],
                kb,
            );
            let t = vertex(
                m,
                at(j, c.radius, c.y[j] + c.kerb[j]),
                out(j),
                kerb_colour,
                half,
                [half - c.kerb[j], along[j]],
                kb,
            );
            (b, t)
        })
        .collect();
    for j in 0..n - 1 {
        let ((b0, t0), (b1, t1)) = (rows[j], rows[j + 1]);
        let face = out(j) + out(j + 1);
        tri(m, b0, t0, b1, face);
        tri(m, t0, t1, b1, face);
    }
    // The sidewalk, rising outwards from the kerb.
    let sb = code_b(sidewalk_layer, Kind::Sidewalk, 0);
    let walk: Vec<(u32, u32)> = (0..n)
        .map(|j| {
            let top = c.y[j] + c.kerb[j];
            let w = c.width[j];
            let i = vertex(
                m,
                at(j, c.radius, top),
                Vec3::Y,
                sidewalk_colour,
                half,
                [half, along[j]],
                sb,
            );
            let o = vertex(
                m,
                at(j, c.radius - w, top + w * SIDEWALK_FALL),
                Vec3::Y,
                sidewalk_colour,
                half,
                [half + w, along[j]],
                sb,
            );
            (i, o)
        })
        .collect();
    for j in 0..n - 1 {
        let ((i0, o0), (i1, o1)) = (walk[j], walk[j + 1]);
        tri(m, i0, o0, i1, Vec3::Y);
        tri(m, o0, o1, i1, Vec3::Y);
    }
    // A skirt under the outer edge (towards the block).
    let wb = code_b(KERB_LAYER, Kind::Wall, 0);
    let skirt: Vec<(u32, u32)> = (0..n)
        .map(|j| {
            let w = c.width[j];
            let r = c.radius - w;
            let top = c.y[j] + c.kerb[j] + w * SIDEWALK_FALL;
            let t = vertex(
                m,
                at(j, r, top),
                -out(j),
                wall_colour,
                half,
                [half + w, along[j]],
                wb,
            );
            let b = vertex(
                m,
                at(j, r, c.y[j] - SKIRT_M),
                -out(j),
                wall_colour,
                half,
                [half + w + 0.03, along[j]],
                wb,
            );
            (t, b)
        })
        .collect();
    for j in 0..n - 1 {
        let ((t0, b0), (t1, b1)) = (skirt[j], skirt[j + 1]);
        let face = -(out(j) + out(j + 1));
        tri(m, t0, b0, t1, face);
        tri(m, b0, b1, t1, face);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::graph::RoadClass;

    /// A residential street from the junction at `(x, z)` in direction
    /// `dir` (degrees from +x towards +z), 40 m, with 2 m sidewalks on
    /// both sides and a 5 m junction zone at its start.
    fn arm(at: Vec2, deg: f32) -> RoadStrip {
        let d = Vec2::from_angle(deg.to_radians());
        let mut dress = RoadDress::of_class(RoadClass::Residential, 6.0);
        dress.sidewalk_m = [2.0, 2.0];
        dress.kerb_m = 0.12;
        dress.start_m = 0.0;
        dress.edge_m = 40.0;
        dress.junction_m = [5.0, 0.0];
        RoadStrip {
            class: RoadClass::Residential,
            width_m: 6.0,
            points: (0..=8)
                .map(|i| {
                    let p = at + d * (i as f32 * 5.0);
                    [p.x, 10.0, p.y]
                })
                .collect(),
            surface: ocw_world::RoadSurface::Asphalt,
            dress,
            lit: ocw_world::RoadLit::Unknown,
        }
    }

    /// The pilot package (`OCW_CORNER_WORLD=…/liechtenstein.ocwpkg`;
    /// skipped without): its town junctions get round corners. Prints the
    /// arms near the Äulestrasse junction in Vaduz.
    #[test]
    fn pilot_town_junctions_get_round_corners() {
        let Some(path) = std::env::var_os("OCW_CORNER_WORLD") else {
            eprintln!("OCW_CORNER_WORLD not set: skipped");
            return;
        };
        let pkg = ocw_world::WorldPackage::from_bytes(&std::fs::read(path).unwrap()).unwrap();
        let (mut total, mut with_sidewalks, mut arm_count) = (0, 0, 0);
        for tile in pkg.tiles.values() {
            total += corners(&tile.roads).len();
            with_sidewalks += tile
                .roads
                .iter()
                .filter(|s| s.dress.sidewalk_m.iter().any(|w| *w > 0.0))
                .count();
            arm_count += arms(&tile.roads).len();
        }
        eprintln!("corners {total}, strips with sidewalks {with_sidewalks}, arms {arm_count}");
        let at = Vec2::new(-2421.0, 2300.0);
        let key = ocw_world::TileKey::containing(
            [f64::from(at.x), f64::from(at.y)],
            f64::from(pkg.tile_size_m),
        );
        let tile = &pkg.tiles[&key];
        for (i, s) in tile.roads.iter().enumerate() {
            let (a, b) = (
                horizontal(&s.points[0]),
                horizontal(&s.points[s.points.len() - 1]),
            );
            if a.distance(at) < 60.0 || b.distance(at) < 60.0 {
                let arc = arc_lengths(s);
                eprintln!(
                    "strip {i}: {:?} w {} start {} edge {} len {} junction {:?} sidewalks {:?} from {a} to {b}",
                    s.class,
                    s.width_m,
                    s.dress.start_m,
                    s.dress.edge_m,
                    arc[arc.len() - 1],
                    s.dress.junction_m,
                    s.dress.sidewalk_m
                );
            }
        }
        for a in arms(&tile.roads) {
            if a.at.distance(at) < 60.0 {
                eprintln!("arm {a:?}");
            }
        }
        for c in corners(&tile.roads) {
            if c.p.distance(at) < 60.0 {
                eprintln!("corner at {} r {} ends {:?}", c.p, c.radius, c.ends);
            }
        }
        // The corners nearest to Vaduz centre (for scenes).
        let centre = Vec2::new(-2430.0, 2220.0);
        let mut near: Vec<(f32, Vec2, f32)> = pkg
            .tiles
            .values()
            .flat_map(|t| corners(&t.roads))
            .map(|c| (c.p.distance(centre), c.p, c.radius))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (d, p, r) in near.iter().take(12) {
            eprintln!("near centre: {d:.0} m at {p} r {r:.1}");
        }
        assert!(total > 50, "{total}");
    }

    /// A crossroads of two streets: four corners, each a quarter circle
    /// tangent to both kerb lines, the strips' sidewalks starting at the
    /// tangent points.
    #[test]
    fn a_crossroads_has_four_round_corners() {
        let j = Vec2::new(100.0, 100.0);
        let strips: Vec<RoadStrip> = [0.0, 90.0, 180.0, 270.0].map(|a| arm(j, a)).into();
        let cs = corners(&strips);
        assert_eq!(cs.len(), 4, "{cs:?}");
        for c in &cs {
            // A quarter circle (90° between the streets).
            let a = (c.arc[0] - c.centre).angle_to(c.arc[c.arc.len() - 1] - c.centre);
            assert!((a.abs() - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "{a}");
            // On the circle, outside both carriageways, nearer the
            // junction than the centre.
            for q in &c.arc {
                assert!((q.distance(c.centre) - c.radius).abs() < 1e-3);
                let rel = *q - j;
                assert!(
                    rel.x.abs() >= 3.0 - 1e-3 && rel.y.abs() >= 3.0 - 1e-3,
                    "{q}"
                );
            }
            // The kerb lines cross at (±3, ±3) from the junction.
            let pr = c.p - j;
            assert!((pr.x.abs() - 3.0).abs() < 1e-3 && (pr.y.abs() - 3.0).abs() < 1e-3);
            // Radius: the tangent points in the kerb ramps (zone 5 + 1 −
            // half width 3 = 3 m), at least the sidewalk + 0.5 m.
            assert!((c.radius - 3.0).abs() < 1e-3, "{}", c.radius);
            assert_eq!(c.ends.len(), 2);
            for &(_, _, _, t) in &c.ends {
                assert!((t - 6.0).abs() < 1e-3, "{t}");
            }
            assert!(c.width.iter().all(|w| (*w - 2.0).abs() < 1e-6));
        }
        // Every strip's both sidewalks start at the corner.
        let tr = trims(&cs, strips.len());
        for t in &tr {
            assert_eq!(t[0][0], Some(6.0));
            assert_eq!(t[1][0], Some(6.0));
            assert_eq!(t[0][1], None);
        }
        // Trimmed: nothing before the start, the full width right after it
        // (no wedge), the junction taper gone at that end.
        let d = &strips[0].dress;
        assert_eq!(kerb_trimmed(d, 0, 5.9, 40.0, &tr[0][0]), (0.0, 0.0));
        assert_eq!(kerb_trimmed(d, 0, 6.0, 40.0, &tr[0][0]), (2.0, 0.12));
        // Untrimmed, the old taper: half width in the middle of the ramp.
        assert_eq!(d.kerb_at(0, 6.0), (1.0, 0.06));
        // The cut points: inserted at 5.98 and 6.0 m.
        let cut = with_cuts(&strips[0], &tr[0]);
        let arc = arc_lengths(&cut);
        assert!(arc.iter().any(|a| (a - 5.98).abs() < 1e-4));
        assert!(arc.iter().any(|a| (a - 6.0).abs() < 1e-4));
        assert!((arc[arc.len() - 1] - 40.0).abs() < 1e-3);
    }

    /// Only facing sidewalks get a corner; a straight road through a T
    /// junction (180°) and roads without sidewalks keep the taper; a strip
    /// end that is no junction is no arm.
    #[test]
    fn corners_only_where_two_sidewalks_meet() {
        let j = Vec2::new(0.0, 0.0);
        // A T: east, west, and south (+z).
        let mut strips: Vec<RoadStrip> = [0.0, 180.0, 90.0].map(|a| arm(j, a)).into();
        assert_eq!(corners(&strips).len(), 2, "east–south and south–west");
        // The south street without its left sidewalk (its left is east,
        // seen from the junction): the east–south corner goes.
        strips[2].dress.sidewalk_m = [0.0, 2.0];
        assert_eq!(corners(&strips).len(), 1);
        // No junction zones: no arms.
        for s in &mut strips {
            s.dress.junction_m = [0.0, 0.0];
        }
        assert!(corners(&strips).is_empty());
        // 45° apart: a corner with a tighter angle and a larger offset.
        let cs = corners(&[arm(j, 0.0), arm(j, 45.0)]);
        assert_eq!(cs.len(), 1, "{cs:?}");
        let c = &cs[0];
        let a = (c.arc[0] - c.centre).angle_to(c.arc[c.arc.len() - 1] - c.centre);
        assert!((a.abs() - 135f32.to_radians()).abs() < 1e-3, "{a}");
    }

    /// A town street split 4 m after the junction (a crossing node): the
    /// arc's tangent point lies on the next strip, which continues it – the
    /// short strip loses that sidewalk, the next one starts it at the arc.
    #[test]
    fn a_short_arm_runs_on_into_the_strip_that_continues_it() {
        let j = Vec2::new(0.0, 0.0);
        let east = arm(j, 0.0);
        let mut short = arm(j, 90.0);
        short.points.truncate(2);
        short.points[1] = [0.0, 10.0, 4.0];
        short.dress.edge_m = 4.0;
        let mut next = arm(Vec2::new(0.0, 4.0), 90.0);
        next.dress.junction_m = [0.0, 0.0];
        let strips = vec![east, short, next];
        let cs = corners(&strips);
        assert_eq!(cs.len(), 1, "{cs:?}");
        let c = &cs[0];
        assert!((c.radius - 3.0).abs() < 1e-3);
        // Between the east street's right side (+z is right of +x) and
        // the short arm's left.
        let mut ends = c.ends.clone();
        ends.sort_by_key(|e| e.0);
        assert_eq!(ends.len(), 3, "{ends:?}");
        assert_eq!((ends[0].0, ends[0].1, ends[0].2), (0, 0, 1));
        assert!((ends[0].3 - 6.0).abs() < 1e-3);
        // The short strip: none left on its left side.
        assert_eq!((ends[1].0, ends[1].1, ends[1].2), (1, 0, 0));
        assert!(ends[1].3 > 4.0);
        // The next strip: from 2 m on (6 m from the junction).
        assert_eq!((ends[2].0, ends[2].1, ends[2].2), (2, 0, 0));
        assert!((ends[2].3 - 2.0).abs() < 1e-3, "{:?}", ends[2]);
        let tr = trims(&cs, strips.len());
        assert_eq!(
            kerb_trimmed(&strips[1].dress, 0, 3.9, 4.0, &tr[1][0]),
            (0.0, 0.0)
        );
        assert_eq!(
            kerb_trimmed(&strips[2].dress, 0, 1.9, 40.0, &tr[2][0]),
            (0.0, 0.0)
        );
        assert_eq!(
            kerb_trimmed(&strips[2].dress, 0, 2.0, 40.0, &tr[2][0]),
            (2.0, 0.12)
        );
        // Without the continuation: no corner (the taper as before).
        assert!(corners(&strips[..2]).is_empty());
    }

    /// The corner's mesh: the sidewalk faces up at kerb height above the
    /// road, the kerb face looks at the road, the road surface lies inside
    /// the arc at road height.
    #[test]
    fn the_corner_mesh_has_kerb_sidewalk_and_road() {
        let j = Vec2::new(100.0, 100.0);
        let strips: Vec<RoadStrip> = [0.0, 90.0].map(|a| arm(j, a)).into();
        let cs = corners(&strips);
        assert_eq!(cs.len(), 1);
        let mut m = MeshArrays::default();
        let st = ocw_world::road_dress::MarkingStyle::default();
        let ctx = RoadContext {
            colours: None,
            ground: &|_, _| Some(9.9),
            deck: &|_| false,
            style: &st,
        };
        corner_mesh(&mut m, &cs[0], &strips, &ctx);
        let road = 10.0 + crate::ROAD_SURFACE_LIFT_M;
        for t in m.indices.chunks(3) {
            let p = |i: u32| Vec3::from(m.positions[i as usize]);
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let n = (b - a).cross(c - a);
            let kind = crate::road_mesh::decode_b(m.uvs_b[t[0] as usize][0]).1;
            if kind == Kind::Sidewalk as u8 || kind == Kind::Surface as u8 {
                assert!(n.y > 0.0, "faces up");
            }
            if kind == Kind::Kerb as u8 {
                // Away from the arc's centre: towards the road.
                let mid = (a + b + c) / 3.0;
                let out = Vec3::new(mid.x - cs[0].centre.x, 0.0, mid.z - cs[0].centre.y);
                assert!(n.dot(out) > 0.0, "kerb faces the road");
            }
        }
        for (q, b) in m.positions.iter().zip(&m.uvs_b) {
            let kind = crate::road_mesh::decode_b(b[0]).1;
            if kind == Kind::Sidewalk as u8 {
                assert!(
                    q[1] >= road + 0.12 - 1e-4 && q[1] <= road + 0.12 + 0.05,
                    "{q:?}"
                );
            }
            if kind == Kind::Surface as u8 {
                assert!((q[1] - road).abs() < 1e-4, "{q:?}");
            }
        }
    }
}
