//! The look of tunnels and galleries (spec 3.2 *Tunnels*, 2026-11-28):
//! meshes for the [`SectionKind`]s of a tunnel ([`ocw_world::tunnels`])
//! along its axis, and the one small texture they share.
//!
//! - **Bored:** a vault of sprayed concrete (shotcrete grain, panel joints
//!   every 10 m), the lower 1.6 m of the walls painted light, soot darker
//!   towards the crown above the lanes; raised walkways with kerbs, the
//!   road markings carried on; a cable tray along the vault with lamps
//!   every 12 m (every 4 m in the 40 m entrance zones, as real tunnels
//!   light their entrances more for the eye coming from daylight); the
//!   light pools on the road and walls are baked into the vertex colours
//!   (the tube is unlit: sunlight and tree shadows would reach through
//!   the hill); reflectors every 25 m, emergency niches with SOS signs
//!   every 150 m, jet fans every 200 m in tunnels over 400 m; a concrete
//!   portal frame at each end.
//! - **Gallery:** lit concrete: the wall on the hill side, a roof slab,
//!   pillars and a parapet on the open valley side, lamps under the roof.
//! - **Open:** the road surface with its markings and kerbs.
//!
//! The texture ([`tunnel_texture_pixels`], 256 × 256 RGBA8 with its mip
//! chain: 349,524 bytes) is a tileable grey: shotcrete grain, faint
//! streaks along the tunnel and a joint line; UVs are in metres along and
//! around the tube (one repeat per [`TEX_REPEAT_M`]), and a slow noise in
//! the vertex colours hides the repetition.

use bevy::prelude::*;
use ocw_world::tunnels::{
    self as tn, AxisSample, BOX_ROOF_M, CROWN_M, GALLERY_PILLAR_STEP_M, GALLERY_ROOF_M,
    GALLERY_SLAB_M, SectionKind, WALL_M,
};

use crate::package::MeshArrays;

/// Size of the tunnel texture (square, px).
pub const TEX_SIZE: u32 = 256;
/// One texture repeat covers this many metres.
pub const TEX_REPEAT_M: f32 = 4.0;
/// Lamps along the vault (m), and in the entrance zones.
pub const LAMP_STEP_M: f64 = 12.0;
pub const ENTRANCE_LAMP_STEP_M: f64 = 4.0;
pub const ENTRANCE_ZONE_M: f64 = 40.0;
/// Painted band at the foot of the walls (m).
pub const LIGHT_BAND_M: f64 = 1.6;
/// Walkway (kerb) height beside the road (m).
pub const WALKWAY_M: f32 = 0.2;
/// The tube reaches this far out of a bored section's end (the portal's
/// collar, m).
pub const COLLAR_M: f64 = 1.5;
const REFLECTOR_STEP_M: f64 = 25.0;
const NICHE_STEP_M: f64 = 150.0;
const FAN_STEP_M: f64 = 200.0;
const FAN_MIN_TUNNEL_M: f64 = 400.0;

/// The meshes of one tunnel: the tube (unlit, textured), its fittings
/// (unlit, plain), what glows (lamps, signs, reflectors) and the lit
/// concrete (portals, galleries, open road).
#[derive(Default)]
pub struct TunnelMeshes {
    pub tube: MeshArrays,
    pub fittings: MeshArrays,
    pub glow: MeshArrays,
    pub lit: MeshArrays,
}

impl TunnelMeshes {
    pub fn is_empty(&self) -> bool {
        self.tube.indices.is_empty()
            && self.fittings.indices.is_empty()
            && self.glow.indices.is_empty()
            && self.lit.indices.is_empty()
    }

    /// Vertices of all parts.
    pub fn vertices(&self) -> usize {
        [&self.tube, &self.fittings, &self.glow, &self.lit]
            .iter()
            .map(|m| m.positions.len())
            .sum()
    }
}

/// One point of a tunnel's axis: the road surface's centre, the right
/// unit vector (x, z) and the road width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisPoint {
    pub s: f64,
    pub pos: Vec3,
    pub right: Vec2,
    pub width: f64,
}

impl AxisPoint {
    /// From a sample of [`ocw_world::tunnels::probe`]; `lift` raises the
    /// road surface (the drawn road lies above the profile).
    pub fn from_sample(x: &AxisSample, width: f64, lift: f32) -> Self {
        Self {
            s: x.s,
            pos: Vec3::new(x.p[0] as f32, x.road_y as f32 + lift, x.p[1] as f32),
            right: Vec2::new(-x.dir[1] as f32, x.dir[0] as f32),
            width,
        }
    }

    fn half(&self) -> f32 {
        tn::half_width(self.width) as f32
    }

    /// The point at lateral `l` (right positive) and height `h`.
    fn at(&self, l: f32, h: f32) -> Vec3 {
        self.pos + Vec3::new(self.right.x * l, h, self.right.y * l)
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(self.right.y, 0.0, -self.right.x)
    }
}

/// Deterministic value noise in 0 … 1 (integer lattice, smooth).
fn hash01(i: i64, seed: u64) -> f32 {
    let mut h =
        (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 32;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 29;
    (h & 0xFFFF) as f32 / 65535.0
}

fn noise1(x: f64, seed: u64) -> f32 {
    let i = x.floor();
    let t = (x - i) as f32;
    let t = t * t * (3.0 - 2.0 * t);
    let (a, b) = (hash01(i as i64, seed), hash01(i as i64 + 1, seed));
    a + (b - a) * t
}

/// Light on the tube at `s` (0 … 1): the pools under the lamps.
pub fn lamp_light(s: f64, lamps: &[f64]) -> f32 {
    // Lamps are sorted: look at the neighbours only.
    let i = lamps.partition_point(|&l| l < s);
    let mut best: f32 = 0.0;
    for j in i.saturating_sub(2)..(i + 2).min(lamps.len()) {
        let d = ((s - lamps[j]) / 4.5) as f32;
        best = best.max((-d * d).exp());
    }
    best
}

/// The lamp positions (distance along) of a bored stretch `from` … `to`:
/// every [`LAMP_STEP_M`], every [`ENTRANCE_LAMP_STEP_M`] within
/// [`ENTRANCE_ZONE_M`] of each end.
pub fn lamp_positions(from: f64, to: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut s = from + 2.0;
    while s <= to - 1.0 {
        out.push(s);
        let entrance = s - from < ENTRANCE_ZONE_M || to - s < ENTRANCE_ZONE_M;
        s += if entrance {
            ENTRANCE_LAMP_STEP_M
        } else {
            LAMP_STEP_M
        };
    }
    out
}

struct Builder<'a> {
    m: &'a mut MeshArrays,
}

impl Builder<'_> {
    fn quad(&mut self, v: [Vec3; 4], uv: [[f32; 2]; 4], c: [[f32; 4]; 4]) {
        // v: a, b (one ring), c, d (the next): a-b-c, b-d-c.
        let n = (v[2] - v[0]).cross(v[1] - v[0]).normalize_or_zero();
        let base = self.m.positions.len() as u32;
        for k in 0..4 {
            self.m.positions.push(v[k].to_array());
            self.m.normals.push(n.to_array());
            self.m.colors.push(c[k]);
            self.m.uvs.push(uv[k]);
        }
        self.m
            .indices
            .extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }

    /// A box from `centre` with half extents along `ax` (forward), `ay`
    /// (up) and `az` (right), all faces.
    fn cuboid(&mut self, centre: Vec3, ax: Vec3, ay: Vec3, az: Vec3, c: [f32; 4]) {
        let p = |sx: f32, sy: f32, sz: f32| centre + ax * sx + ay * sy + az * sz;
        let faces: [[Vec3; 4]; 6] = [
            [
                p(-1., 1., -1.),
                p(-1., 1., 1.),
                p(1., 1., -1.),
                p(1., 1., 1.),
            ],
            [
                p(-1., -1., 1.),
                p(-1., -1., -1.),
                p(1., -1., 1.),
                p(1., -1., -1.),
            ],
            [
                p(-1., -1., -1.),
                p(-1., 1., -1.),
                p(1., -1., -1.),
                p(1., 1., -1.),
            ],
            [
                p(-1., 1., 1.),
                p(-1., -1., 1.),
                p(1., 1., 1.),
                p(1., -1., 1.),
            ],
            [
                p(-1., -1., 1.),
                p(-1., 1., 1.),
                p(-1., -1., -1.),
                p(-1., 1., -1.),
            ],
            [
                p(1., -1., -1.),
                p(1., 1., -1.),
                p(1., -1., 1.),
                p(1., 1., 1.),
            ],
        ];
        for f in faces {
            self.quad(f, [[0.0; 2]; 4], [c; 4]);
        }
    }

    /// A flat panel on a wall: centre, unit `along` and `up`, half sizes.
    fn panel(&mut self, centre: Vec3, along: Vec3, up: Vec3, hw: f32, hh: f32, c: [f32; 4]) {
        let v = [
            centre - along * hw - up * hh,
            centre - along * hw + up * hh,
            centre + along * hw - up * hh,
            centre + along * hw + up * hh,
        ];
        self.quad(v, [[0.0; 2]; 4], [c; 4]);
    }

    /// A cylinder along `axis` (unit) of `radius`, `half_len`, 10 sides.
    fn cylinder(
        &mut self,
        centre: Vec3,
        axis: Vec3,
        radius: f32,
        half_len: f32,
        c: [f32; 4],
        rim: [f32; 4],
    ) {
        let side = axis.cross(Vec3::Y).normalize_or_zero();
        let up = side.cross(axis).normalize_or_zero();
        const N: usize = 10;
        let ring = |k: usize, r: f32| {
            let a = std::f32::consts::TAU * k as f32 / N as f32;
            (side * a.cos() + up * a.sin()) * r
        };
        for k in 0..N {
            let (r0, r1) = (ring(k, radius), ring(k + 1, radius));
            let (a, b) = (centre - axis * half_len, centre + axis * half_len);
            self.quad([a + r0, a + r1, b + r0, b + r1], [[0.0; 2]; 4], [c; 4]);
            // The rims (dark fan face) at both ends.
            let (i0, i1) = (ring(k, radius * 0.85), ring(k + 1, radius * 0.85));
            self.quad([a + i0, a + i1, a + r0, a + r1], [[0.0; 2]; 4], [rim; 4]);
            self.quad([b + i1, b + i0, b + r1, b + r0], [[0.0; 2]; 4], [rim; 4]);
        }
    }
}

fn grey(v: f32) -> [f32; 4] {
    [v, v, v, 1.0]
}

fn scale(c: [f32; 4], f: f32) -> [f32; 4] {
    [c[0] * f, c[1] * f, c[2] * f, c[3]]
}

/// The tube's wall and vault profile from the left foot over the crown
/// to the right foot: (lateral, height, colour key), the walls split at
/// the painted band.
fn vault_profile(half: f32, boxed: bool) -> Vec<(f32, f32)> {
    let (w, c) = (WALL_M as f32, CROWN_M as f32);
    let band = LIGHT_BAND_M as f32;
    let w = if boxed { BOX_ROOF_M as f32 } else { w };
    let mut p = vec![
        (-half, WALKWAY_M),
        (-half, band),
        (-half, band + 0.08),
        (-half, w),
    ];
    const ARCH: usize = 14;
    for k in 1..ARCH {
        if boxed {
            // A flat roof, split as the arch for the soot and the light.
            p.push((half * (-1.0 + 2.0 * k as f32 / ARCH as f32), w));
            continue;
        }
        let a = std::f32::consts::PI * (1.0 - k as f32 / ARCH as f32);
        p.push((half * a.cos(), w + (c - w) * a.sin()));
    }
    p.extend([
        (half, w),
        (half, band + 0.08),
        (half, band),
        (half, WALKWAY_M),
    ]);
    p
}

/// Colour of the vault at lateral `l`, height `h` (before light).
fn vault_colour(l: f32, h: f32, half: f32, road_half: f32) -> [f32; 4] {
    let band = LIGHT_BAND_M as f32;
    if h <= band + 0.01 {
        // The painted band, a little dirty at the foot.
        let v = 0.70 + 0.08 * (h / band);
        return [v, v * 0.99, v * 0.95, 1.0];
    }
    if h <= band + 0.09 {
        return [0.16, 0.16, 0.17, 1.0]; // the dark line over the band
    }
    // Concrete, darker towards the crown and over the lanes (soot).
    let up = ((h - band) / (CROWN_M as f32 - band)).clamp(0.0, 1.0);
    let over_lanes = (1.0 - (l.abs() / road_half.max(0.5)).min(1.0)).powf(0.7);
    let soot = 0.35 * up * up + 0.22 * over_lanes * up;
    let v = 0.47 * (1.0 - soot);
    let _ = half;
    [v * 1.02, v, v * 0.96, 1.0]
}

/// Builds the meshes of one tunnel: its axis points (every
/// [`tn::STEP_M`], from the edge's start) and its sections.
pub fn tunnel_meshes(axis: &[AxisPoint], sections: &[tn::Section], seed: u64) -> TunnelMeshes {
    tunnel_meshes_with(axis, sections, seed, [true, true])
}

/// [`tunnel_meshes`]; `portals` says whether each end of the axis is a
/// portal (false where the tunnel goes on in the next edge of the graph:
/// no frame inside the hill).
pub fn tunnel_meshes_with(
    axis: &[AxisPoint],
    sections: &[tn::Section],
    seed: u64,
    portals: [bool; 2],
) -> TunnelMeshes {
    let mut out = TunnelMeshes::default();
    if axis.len() < 2 {
        return out;
    }
    let kind_at = |s: f64| {
        sections
            .iter()
            .find(|k| s >= k.from - 1e-6 && s <= k.to + 1e-6)
            .map_or(SectionKind::Open, |k| k.kind)
    };
    let total = axis.last().map_or(0.0, |a| a.s);
    // Roofed stretches (bored tubes, boxes) with their collars (into the
    // next section).
    let roofed = |k: SectionKind| matches!(k, SectionKind::Bored | SectionKind::Box);
    let bored: Vec<(f64, f64, bool)> = sections
        .iter()
        .filter(|k| roofed(k.kind))
        .map(|k| (k.from, k.to, k.kind == SectionKind::Box))
        .collect();
    let in_tube = |s: f64| {
        bored
            .iter()
            .find(|&&(a, b, _)| s >= a - 1e-6 && s <= b + 1e-6)
            .or_else(|| {
                bored
                    .iter()
                    .find(|&&(a, b, _)| s >= a - COLLAR_M - 1e-6 && s <= b + COLLAR_M + 1e-6)
            })
            .map(|&(_, _, boxed)| boxed)
    };
    let lamps: Vec<f64> = bored
        .iter()
        .flat_map(|&(a, b, _)| lamp_positions(a, b))
        .collect();
    for w in axis.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mid = (a.s + b.s) / 2.0;
        if let Some(boxed) = in_tube(mid) {
            tube_segment(&mut out, a, b, &lamps, seed, boxed);
        } else {
            match kind_at(mid) {
                SectionKind::Gallery { hill_right } => {
                    gallery_segment(&mut out, a, b, hill_right, seed);
                }
                _ => open_segment(&mut out, a, b),
            }
        }
    }
    let point_at = |s: f64| -> AxisPoint {
        let i = axis.partition_point(|p| p.s < s).clamp(1, axis.len() - 1);
        let (a, b) = (axis[i - 1], axis[i]);
        let t = ((s - a.s) / (b.s - a.s).max(1e-9)).clamp(0.0, 1.0) as f32;
        AxisPoint {
            s,
            pos: a.pos.lerp(b.pos, t),
            right: a.right.lerp(b.right, t).normalize_or_zero(),
            width: a.width,
        }
    };
    // Fittings along the roofed stretches.
    for &(from, to, boxed) in &bored {
        let len = to - from;
        let roof = |l: f32, half: f32| -> f32 {
            if boxed {
                BOX_ROOF_M as f32
            } else {
                tn::roof_height(f64::from(l), f64::from(half)) as f32
            }
        };
        let mut fit = Builder {
            m: &mut out.fittings,
        };
        // The cable tray along the vault, left of the crown.
        let mut s = from;
        while s < to {
            let e = (s + tn::STEP_M * 4.0).min(to);
            let (p, q) = (point_at(s), point_at(e));
            let l = -0.35 * p.half();
            let h = roof(l, p.half()) - 0.18;
            let c = (p.at(l, h) + q.at(l, h)) / 2.0;
            let ax = (q.at(l, h) - p.at(l, h)) / 2.0;
            let r = Vec3::new(p.right.x, 0.0, p.right.y);
            fit.cuboid(c, ax, Vec3::Y * 0.06, r * 0.16, grey(0.12));
            s = e;
        }
        // Jet fans in long tunnels, in pairs under the crown.
        if len >= FAN_MIN_TUNNEL_M && !boxed {
            let mut s = from + 100.0;
            while s < to - 100.0 {
                let p = point_at(s);
                for side in [-1.0f32, 1.0] {
                    let l = side * 0.42 * p.half();
                    let h = tn::roof_height(f64::from(l), f64::from(p.half())) as f32 - 0.95;
                    fit.cylinder(p.at(l, h), p.forward(), 0.55, 1.8, grey(0.36), grey(0.06));
                }
                s += FAN_STEP_M;
            }
        }
        // Emergency niches (dark recesses) on the right wall.
        let mut s = from + NICHE_STEP_M / 2.0;
        while s < to - 20.0 {
            let p = point_at(s);
            let c = p.at(p.half() - 0.02, WALKWAY_M + 1.1);
            fit.panel(c, p.forward(), Vec3::Y, 0.7, 1.05, grey(0.09));
            s += NICHE_STEP_M;
        }
        // Glowing: lamps, SOS signs over the niches, reflectors.
        let mut glow = Builder { m: &mut out.glow };
        for &s in lamps.iter().filter(|&&s| s >= from && s <= to) {
            let p = point_at(s);
            let l = -0.35 * p.half();
            let h = roof(l, p.half()) - 0.27;
            let r = Vec3::new(p.right.x, 0.0, p.right.y);
            glow.cuboid(
                p.at(l, h),
                p.forward() * 0.6,
                Vec3::Y * 0.04,
                r * 0.11,
                [1.0, 0.88, 0.66, 1.0],
            );
        }
        let mut s = from + NICHE_STEP_M / 2.0;
        while s < to - 20.0 {
            let p = point_at(s);
            let c = p.at(p.half() - 0.04, WALKWAY_M + 2.55);
            glow.panel(c, p.forward(), Vec3::Y, 0.32, 0.2, [0.55, 0.78, 1.0, 1.0]);
            s += NICHE_STEP_M;
        }
        let mut s = from + REFLECTOR_STEP_M / 2.0;
        while s < to {
            let p = point_at(s);
            for side in [-1.0f32, 1.0] {
                let c = p.at(side * (p.half() - 0.03), WALKWAY_M + 0.75);
                glow.panel(c, p.forward(), Vec3::Y, 0.06, 0.06, [0.95, 0.55, 0.12, 1.0]);
            }
            s += REFLECTOR_STEP_M;
        }
        // The portal frames at both ends (where the tube leaves the hill);
        // where a tube meets a box inside, a bulkhead closes the arch over
        // the box's roof.
        for (s, out_dir) in [(from - COLLAR_M, -1.0f32), (to + COLLAR_M, 1.0)] {
            if s < -COLLAR_M - 0.5 || s > total + COLLAR_M + 0.5 {
                continue;
            }
            if (out_dir < 0.0 && from <= 0.5 && !portals[0])
                || (out_dir > 0.0 && to >= total - 0.5 && !portals[1])
            {
                continue;
            }
            let beyond = kind_at((s + f64::from(out_dir) * 1.0).clamp(0.0, total));
            let at = if out_dir < 0.0 { from } else { to };
            if roofed(beyond)
                && (s + f64::from(out_dir) * 1.0) > 0.0
                && (s + f64::from(out_dir)) < total
            {
                if !boxed && beyond == SectionKind::Box {
                    bulkhead(&mut out.fittings, &point_at(at));
                }
                continue;
            }
            let p = point_at(s.clamp(0.0, total));
            portal_frame(&mut out.lit, &p, out_dir, s.clamp(0.0, total) as f32, boxed);
        }
    }
    out
}

/// The road surface across a segment from `a` to `b`, lanes, kerbs and
/// walkways to `half`, with markings; `light` per end scales the
/// colours.
fn road_surface(
    m: &mut MeshArrays,
    a: AxisPoint,
    b: AxisPoint,
    half: f32,
    light: [f32; 2],
    walkway: bool,
) {
    let rh = (a.width / 2.0) as f32;
    let asphalt = [0.11, 0.11, 0.115, 1.0];
    let kerb = grey(0.42);
    let mut bl = Builder { m };
    let strip = |bl: &mut Builder, l0: f32, h0: f32, l1: f32, h1: f32, c: [f32; 4]| {
        // From right to left, so the normal faces up.
        let v = [a.at(l1, h1), a.at(l0, h0), b.at(l1, h1), b.at(l0, h0)];
        let tm = TEX_REPEAT_M;
        let uv = [
            [a.s as f32 / tm, l1 / tm],
            [a.s as f32 / tm, l0 / tm],
            [b.s as f32 / tm, l1 / tm],
            [b.s as f32 / tm, l0 / tm],
        ];
        let (ca, cb) = (scale(c, light[0]), scale(c, light[1]));
        bl.quad(v, uv, [ca, ca, cb, cb]);
    };
    strip(&mut bl, -rh, 0.0, rh, 0.0, asphalt);
    if walkway {
        let wk = WALKWAY_M;
        strip(&mut bl, -half, wk, -rh, wk, kerb);
        strip(&mut bl, -rh, wk, -rh, 0.0, kerb);
        strip(&mut bl, rh, 0.0, rh, wk, kerb);
        strip(&mut bl, rh, wk, half, wk, kerb);
    } else {
        // Skirts down to the terrain laid 0.3 m below the road.
        strip(&mut bl, -rh - 0.3, -0.5, -rh, 0.0, kerb);
        strip(&mut bl, rh, 0.0, rh + 0.3, -0.5, kerb);
    }
    // Markings: edge lines, a dashed centre line on two-lane roads.
    let white = [0.82, 0.82, 0.80, 1.0];
    let lift = 0.006;
    for l in [-rh + 0.3, rh - 0.3] {
        strip(&mut bl, l - 0.075, lift, l + 0.075, lift, white);
    }
    if a.width >= 5.5 {
        let dash = (a.s / 9.0).fract() < 0.34;
        if dash {
            strip(&mut bl, -0.06, lift, 0.06, lift, white);
        }
    }
}

fn tube_segment(
    out: &mut TunnelMeshes,
    a: AxisPoint,
    b: AxisPoint,
    lamps: &[f64],
    seed: u64,
    boxed: bool,
) {
    let half = a.half();
    let rh = (a.width / 2.0) as f32;
    let light = |s: f64| {
        let pool = lamp_light(s, lamps);
        let slow = 0.9 + 0.1 * noise1(s / 7.0, seed);
        (0.42 + 0.58 * pool) * slow
    };
    let (la, lb) = (light(a.s), light(b.s));
    // Floor and markings (light pools on the road a little stronger).
    road_surface(
        &mut out.tube,
        a,
        b,
        half,
        [0.35 + 0.75 * (la - 0.42), 0.35 + 0.75 * (lb - 0.42)],
        true,
    );
    // Walls and vault.
    let prof = vault_profile(half, boxed);
    let mut arc = vec![0.0f32];
    for w in prof.windows(2) {
        let d = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        arc.push(arc.last().copied().unwrap_or(0.0) + d);
    }
    let tm = TEX_REPEAT_M;
    let mut bl = Builder { m: &mut out.tube };
    for k in 0..prof.len() - 1 {
        let ((l0, h0), (l1, h1)) = (prof[k], prof[k + 1]);
        // The band's line is its own colour; elsewhere per vertex.
        let c0 = vault_colour(l0, h0, half, rh);
        let mut c1 = vault_colour(l1, h1, half, rh);
        let mut c0b = c0;
        if (h0 - (LIGHT_BAND_M as f32)).abs() < 0.01 && (h1 - h0) > 0.05 && l0 == l1 {
            c0b = [0.16, 0.16, 0.17, 1.0];
            c1 = c0b;
        }
        if (h1 - (LIGHT_BAND_M as f32)).abs() < 0.01 && (h0 - h1) > 0.05 && l0 == l1 {
            c1 = [0.16, 0.16, 0.17, 1.0];
            c0b = c1;
        }
        // Light: the lamps hang left of the crown, so the left wall and
        // the road get more than the right.
        let side = |l: f32| 1.0 - 0.12 * (l / half);
        let v = [a.at(l0, h0), a.at(l1, h1), b.at(l0, h0), b.at(l1, h1)];
        let uv = [
            [a.s as f32 / tm, arc[k] / tm],
            [a.s as f32 / tm, arc[k + 1] / tm],
            [b.s as f32 / tm, arc[k] / tm],
            [b.s as f32 / tm, arc[k + 1] / tm],
        ];
        bl.quad(
            v,
            uv,
            [
                scale(c0b, la * side(l0)),
                scale(c1, la * side(l1)),
                scale(c0b, lb * side(l0)),
                scale(c1, lb * side(l1)),
            ],
        );
    }
}

fn gallery_segment(
    out: &mut TunnelMeshes,
    a: AxisPoint,
    b: AxisPoint,
    hill_right: bool,
    seed: u64,
) {
    let half = a.half();
    road_surface(&mut out.lit, a, b, half, [1.0, 1.0], true);
    let sg: f32 = if hill_right { 1.0 } else { -1.0 };
    let roof = GALLERY_ROOF_M as f32;
    let slab = GALLERY_SLAB_M as f32;
    let concrete = |s: f64| {
        let v = 0.58 * (0.9 + 0.1 * noise1(s / 9.0, seed));
        [v * 1.02, v, v * 0.95, 1.0]
    };
    let (ca, cb) = (concrete(a.s), concrete(b.s));
    let tm = TEX_REPEAT_M;
    let mut bl = Builder { m: &mut out.lit };
    let face = |bl: &mut Builder, l0: f32, h0: f32, l1: f32, h1: f32, f: f32| {
        let v = [a.at(l0, h0), a.at(l1, h1), b.at(l0, h0), b.at(l1, h1)];
        let uv = [
            [a.s as f32 / tm, (h0 + l0) / tm],
            [a.s as f32 / tm, (h1 + l1) / tm],
            [b.s as f32 / tm, (h0 + l0) / tm],
            [b.s as f32 / tm, (h1 + l1) / tm],
        ];
        let (x, y) = (scale(ca, f), scale(cb, f));
        bl.quad(v, uv, [x, x, y, y]);
    };
    // The hill-side wall (inner face) and the slab: underside, edge, top.
    let wall = sg * half;
    let valley = -sg * (half + 0.6);
    // Inner faces run from the road side outwards so the normal faces in.
    if hill_right {
        face(&mut bl, wall, WALKWAY_M, wall, roof, 0.95);
        face(&mut bl, wall, roof, valley, roof, 0.7);
        face(&mut bl, valley, roof, valley, roof + slab, 0.9);
        face(&mut bl, valley, roof + slab, wall + 0.4, roof + slab, 1.0);
    } else {
        face(&mut bl, wall, roof, wall, WALKWAY_M, 0.95);
        face(&mut bl, valley, roof, wall, roof, 0.7);
        face(&mut bl, valley, roof + slab, valley, roof, 0.9);
        face(&mut bl, wall - 0.4, roof + slab, valley, roof + slab, 1.0);
    }
    // Pillars and the parapet on the valley side.
    let pl = -sg * (half - 0.1);
    let r = Vec3::new(a.right.x, 0.0, a.right.y);
    let step = GALLERY_PILLAR_STEP_M;
    let ka = (a.s / step).floor();
    let kb = (b.s / step).floor();
    if kb > ka {
        let s = kb * step;
        let t = ((s - a.s) / (b.s - a.s).max(1e-9)) as f32;
        let p = AxisPoint {
            s,
            pos: a.pos.lerp(b.pos, t),
            ..a
        };
        let c = p.at(pl, roof / 2.0);
        bl.cuboid(
            c,
            p.forward() * 0.3,
            Vec3::Y * (roof / 2.0),
            r * 0.3,
            scale(ca, 0.92),
        );
    }
    let par = (b.pos - a.pos) / 2.0;
    let c = (a.at(pl, 0.55) + b.at(pl, 0.55)) / 2.0;
    bl.cuboid(c, par, Vec3::Y * 0.45, r * 0.15, scale(ca, 0.85));
    // Lamps under the slab every 15 m.
    if (a.s / 15.0).floor() < (b.s / 15.0).floor() {
        let mut g = Builder { m: &mut out.glow };
        g.cuboid(
            a.at(0.0, roof - 0.06),
            a.forward() * 0.5,
            Vec3::Y * 0.04,
            r * 0.12,
            [1.0, 0.9, 0.7, 1.0],
        );
    }
}

fn open_segment(out: &mut TunnelMeshes, a: AxisPoint, b: AxisPoint) {
    road_surface(&mut out.lit, a, b, a.half(), [1.0, 1.0], false);
}

/// A concrete frame round the tube's mouth at `p`, facing `out_dir`
/// along the axis (−1 at the start, +1 at the end).
fn portal_frame(m: &mut MeshArrays, p: &AxisPoint, out_dir: f32, s: f32, boxed: bool) {
    let half = p.half();
    let prof = vault_profile(half, boxed);
    let fwd = p.forward() * out_dir;
    let depth = 0.6;
    let c = grey(0.55);
    let mut bl = Builder { m };
    let rim = 0.9f32;
    for k in 0..prof.len() - 1 {
        let ((l0, h0), (l1, h1)) = (prof[k], prof[k + 1]);
        // Outer points: pushed out from the tube's centre line by `rim`
        // (laterally on the walls, up and out on the arch).
        let outer = |l: f32, h: f32| {
            let dir = Vec2::new(
                l,
                (h - WALL_M as f32).max(0.0) * (half / (CROWN_M - WALL_M) as f32),
            )
            .normalize_or_zero();
            let (ol, oh) = if h < WALL_M as f32 {
                (l + l.signum() * rim, h)
            } else {
                (l + dir.x * rim, h + dir.y * rim)
            };
            (ol, oh)
        };
        let (o0, o1) = (outer(l0, h0), outer(l1, h1));
        let front = |l: f32, h: f32| p.at(l, h) + fwd * depth;
        let back = |l: f32, h: f32| p.at(l, h);
        let uv = [
            [s / 4.0, 0.0],
            [s / 4.0, 0.25],
            [s / 4.0 + 0.25, 0.0],
            [s / 4.0 + 0.25, 0.25],
        ];
        // The face (both orders: seen from outside; double-sided material).
        bl.quad(
            [
                front(l0, h0),
                front(l1, h1),
                front(o0.0, o0.1),
                front(o1.0, o1.1),
            ],
            uv,
            [c; 4],
        );
        // The outer rim's top and the inner reveal.
        bl.quad(
            [
                back(o0.0, o0.1),
                back(o1.0, o1.1),
                front(o0.0, o0.1),
                front(o1.0, o1.1),
            ],
            uv,
            [scale(c, 0.9); 4],
        );
        bl.quad(
            [front(l0, h0), front(l1, h1), back(l0, h0), back(l1, h1)],
            uv,
            [scale(c, 0.8); 4],
        );
    }
}

/// Closes a tube's arch above a box's flat roof where the two meet.
fn bulkhead(m: &mut MeshArrays, p: &AxisPoint) {
    let half = p.half();
    let roof = BOX_ROOF_M as f32;
    let arch: Vec<(f32, f32)> = vault_profile(half, false)
        .into_iter()
        .filter(|&(_, h)| h >= roof)
        .collect();
    let mut bl = Builder { m };
    let c = grey(0.2);
    for w in arch.windows(2) {
        let ((l0, h0), (l1, h1)) = (w[0], w[1]);
        bl.quad(
            [p.at(l0, roof), p.at(l0, h0), p.at(l1, roof), p.at(l1, h1)],
            [[0.0; 2]; 4],
            [c; 4],
        );
    }
}

/// The tunnel texture: a tileable grey (RGBA8, sRGB), [`TEX_SIZE`]²:
/// shotcrete grain in several octaves, faint streaks along u (the
/// tunnel's length) and a panel joint at u = 0 every 2.5 repeats … one
/// joint per repeat of 4 m (10 m joints would need a larger texture; one
/// darker line per repeat reads as the panel rhythm). Deterministic.
pub fn tunnel_texture_pixels() -> Vec<[u8; 4]> {
    let n = TEX_SIZE as i64;
    // Tileable value noise on a lattice of `cells` per side.
    let lattice = |x: i64, y: i64, cells: i64, seed: u64| {
        hash01(x.rem_euclid(cells) * 7919 + y.rem_euclid(cells), seed)
    };
    let noise = |u: f32, v: f32, cells: i64, seed: u64| {
        let (x, y) = (u * cells as f32, v * cells as f32);
        let (xi, yi) = (x.floor() as i64, y.floor() as i64);
        let (tx, ty) = (x - xi as f32, y - yi as f32);
        let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let a = lattice(xi, yi, cells, seed);
        let b = lattice(xi + 1, yi, cells, seed);
        let c = lattice(xi, yi + 1, cells, seed);
        let d = lattice(xi + 1, yi + 1, cells, seed);
        let ab = a + (b - a) * tx;
        let cd = c + (d - c) * tx;
        ab + (cd - ab) * ty
    };
    let mut out = Vec::with_capacity((n * n) as usize);
    for y in 0..n {
        for x in 0..n {
            let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
            // Grain: rough sprayed concrete.
            let g =
                0.45 * noise(u, v, 64, 1) + 0.3 * noise(u, v, 32, 2) + 0.25 * noise(u, v, 128, 3);
            // Streaks along u: noise stretched (few cells along, many across).
            let streak = {
                let (xs, ys) = (u * 2.0, v * 48.0);
                let (xi, yi) = (xs.floor() as i64, ys.floor() as i64);
                let ty = ys - yi as f32;
                let ty = ty * ty * (3.0 - 2.0 * ty);
                let tx = xs - xi as f32;
                let tx = tx * tx * (3.0 - 2.0 * tx);
                let l = |xx: i64, yy: i64| hash01(xx.rem_euclid(2) * 7919 + yy.rem_euclid(48), 4);
                let a = l(xi, yi) + (l(xi + 1, yi) - l(xi, yi)) * tx;
                let b = l(xi, yi + 1) + (l(xi + 1, yi + 1) - l(xi, yi + 1)) * tx;
                a + (b - a) * ty
            };
            // A joint: a thin darker line at u = 0 (every repeat).
            let du = (u.min(1.0 - u)) * n as f32;
            let joint = if du < 1.2 {
                0.55
            } else if du < 2.2 {
                0.85
            } else {
                1.0
            };
            let val = (0.78 + 0.32 * (g - 0.5) - 0.10 * (streak - 0.5).max(0.0) * 2.0) * joint;
            let b = (val.clamp(0.0, 1.0) * 255.0).round() as u8;
            out.push([b, b, b, 255]);
        }
    }
    out
}

/// The mip chain of a square RGBA8 image (2 × 2 box filter, down to 1 px).
pub fn mip_chain(level0: &[[u8; 4]], size: u32) -> Vec<Vec<[u8; 4]>> {
    let mut levels = vec![level0.to_vec()];
    let mut s = size as usize;
    while s > 1 {
        let prev = levels.last().expect("level");
        let h = s / 2;
        let mut next = Vec::with_capacity(h * h);
        for y in 0..h {
            for x in 0..h {
                let mut acc = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = prev[(2 * y + dy) * s + 2 * x + dx];
                    for c in 0..4 {
                        acc[c] += u32::from(p[c]);
                    }
                }
                next.push(acc.map(|v| ((v + 2) / 4) as u8));
            }
        }
        levels.push(next);
        s = h;
    }
    levels
}

/// Bytes of the texture with its mips (the phone budget, spec 10).
pub fn texture_bytes() -> usize {
    mip_chain(&tunnel_texture_pixels(), TEX_SIZE)
        .iter()
        .map(|l| l.len() * 4)
        .sum()
}

/// The texture as a Bevy image (repeating, trilinear).
pub fn tunnel_image() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{
        Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    };
    let levels = mip_chain(&tunnel_texture_pixels(), TEX_SIZE);
    let data: Vec<u8> = levels.iter().flatten().flatten().copied().collect();
    Image {
        data: Some(data),
        texture_descriptor: TextureDescriptor {
            label: Some("tunnel"),
            size: Extent3d {
                width: TEX_SIZE,
                height: TEX_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            anisotropy_clamp: 4,
            ..default()
        }),
        asset_usage: RenderAssetUsages::RENDER_WORLD,
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::tunnels::Section;

    fn axis(len: f64) -> Vec<AxisPoint> {
        let n = (len / tn::STEP_M) as usize;
        (0..=n)
            .map(|i| {
                let s = i as f64 * tn::STEP_M;
                AxisPoint {
                    s,
                    pos: Vec3::new(s as f32, 100.0, 0.0),
                    right: Vec2::new(0.0, 1.0),
                    width: 7.0,
                }
            })
            .collect()
    }

    fn bounds(m: &MeshArrays) -> (Vec3, Vec3) {
        m.positions.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))),
        )
    }

    #[test]
    fn a_bored_tunnel_has_a_tube_lamps_fans_and_portals() {
        let ax = axis(600.0);
        let secs = [Section {
            from: 0.0,
            to: 600.0,
            kind: SectionKind::Bored,
        }];
        let m = tunnel_meshes(&ax, &secs, 1);
        assert!(!m.tube.indices.is_empty());
        let (lo, hi) = bounds(&m.tube);
        assert!(
            hi.y <= 100.0 + CROWN_M as f32 + 0.01 && lo.y >= 99.99,
            "{lo} {hi}"
        );
        // Lamps: dense at the entrances, sparse inside.
        let l = lamp_positions(0.0, 600.0);
        let gaps: Vec<f64> = l.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(gaps[0], ENTRANCE_LAMP_STEP_M);
        assert!(gaps.contains(&LAMP_STEP_M));
        assert!(!m.glow.indices.is_empty() && !m.fittings.indices.is_empty());
        // Two fan pairs' worth of fitting vertices more than a short one.
        let short = tunnel_meshes(
            &axis(300.0),
            &[Section {
                from: 0.0,
                to: 300.0,
                kind: SectionKind::Bored,
            }],
            1,
        );
        assert!(m.fittings.positions.len() > 2 * short.fittings.positions.len());
        // The portal frames are lit concrete.
        assert!(!m.lit.indices.is_empty());
    }

    #[test]
    fn a_box_has_a_flat_roof_and_a_bulkhead_to_the_tube() {
        let ax = axis(600.0);
        let secs = [
            Section {
                from: 0.0,
                to: 300.0,
                kind: SectionKind::Bored,
            },
            Section {
                from: 300.0,
                to: 600.0,
                kind: SectionKind::Box,
            },
        ];
        let m = tunnel_meshes(&ax, &secs, 3);
        // In the box part nothing rises above its roof.
        let top = 100.0 + BOX_ROOF_M as f32 + 0.01;
        assert!(
            m.tube
                .positions
                .iter()
                .filter(|p| p[0] > 310.0)
                .all(|p| p[1] <= top)
        );
        assert!(m.tube.positions.iter().any(|p| p[0] < 290.0 && p[1] > top));
        // No fans in a box; a bulkhead closes the arch at x 300.
        assert!(
            m.fittings
                .positions
                .iter()
                .filter(|p| p[0] > 310.0)
                .all(|p| p[1] <= top)
        );
        assert!(
            m.fittings
                .positions
                .iter()
                .any(|p| (p[0] - 300.0).abs() < 0.1 && p[1] > top)
        );
        // No portal frame inside, between the two.
        assert!(m.lit.positions.iter().all(|p| p[0] < 10.0 || p[0] > 590.0));
    }

    #[test]
    fn lamps_light_the_road_under_them() {
        let lamps = lamp_positions(0.0, 300.0);
        let mid = 150.0;
        let i = lamps.partition_point(|&l| l < mid);
        let at = lamps[i];
        assert!(lamp_light(at, &lamps) > 0.99);
        assert!(lamp_light(at + LAMP_STEP_M / 2.0, &lamps) < 0.5);
    }

    #[test]
    fn a_gallery_is_open_on_the_valley_side() {
        let ax = axis(60.0);
        let secs = [Section {
            from: 0.0,
            to: 60.0,
            kind: SectionKind::Gallery { hill_right: true },
        }];
        let m = tunnel_meshes(&ax, &secs, 1);
        assert!(m.tube.indices.is_empty());
        let (lo, hi) = bounds(&m.lit);
        // The roof on top; the valley side (left, −z) only pillars and a
        // parapet below the roof: no full wall there.
        assert!(hi.y >= 100.0 + GALLERY_ROOF_M as f32);
        assert!(lo.z < -4.0 && hi.z > 4.0);
        let wall_left = m
            .lit
            .positions
            .chunks(4)
            .filter(|q| {
                q.iter().all(|p| p[2] < -4.0)
                    && q.iter().any(|p| p[1] > 103.0)
                    && q.iter().any(|p| p[1] < 101.0)
            })
            .count();
        let pillars = (60.0 / GALLERY_PILLAR_STEP_M) as usize;
        // Only the pillars' faces reach from the road to the roof on the left.
        assert!(wall_left <= pillars * 6, "{wall_left}");
    }

    #[test]
    fn an_open_section_is_road_only() {
        let m = tunnel_meshes(
            &axis(40.0),
            &[Section {
                from: 0.0,
                to: 40.0,
                kind: SectionKind::Open,
            }],
            1,
        );
        let (_, hi) = bounds(&m.lit);
        assert!(hi.y < 100.1);
        assert!(m.tube.indices.is_empty() && m.glow.indices.is_empty());
    }

    #[test]
    fn the_texture_fits_the_phone_budget_and_tiles() {
        let px = tunnel_texture_pixels();
        assert_eq!(px.len(), (TEX_SIZE * TEX_SIZE) as usize);
        // 256² RGBA8 with its mips: 349,524 bytes (under 0.5 MB).
        assert_eq!(texture_bytes(), 349_524);
        // Tileable: across the seam (last row to first, last column to
        // first) the texture changes no more than between two inner rows.
        let n = TEX_SIZE as usize;
        let diff = |a: [u8; 4], b: [u8; 4]| (i32::from(a[0]) - i32::from(b[0])).abs();
        let row = |k: usize, l: usize| -> i32 {
            (0..n).map(|x| diff(px[k * n + x], px[l * n + x])).sum()
        };
        let col = |k: usize, l: usize| -> i32 {
            (0..n).map(|y| diff(px[y * n + k], px[y * n + l])).sum()
        };
        assert!(
            row(n - 1, 0) <= 2 * row(127, 128) + 3 * n as i32,
            "{} {}",
            row(n - 1, 0),
            row(127, 128)
        );
        assert!(
            col(n - 1, 0) <= 2 * col(127, 128) + 3 * n as i32,
            "{} {}",
            col(n - 1, 0),
            col(127, 128)
        );
        // Not flat: a real grain.
        let mean = px.iter().map(|p| f64::from(p[0])).sum::<f64>() / px.len() as f64;
        let var = px
            .iter()
            .map(|p| (f64::from(p[0]) - mean).powi(2))
            .sum::<f64>()
            / px.len() as f64;
        assert!(var.sqrt() > 6.0, "{}", var.sqrt());
    }

    #[test]
    fn meshes_are_deterministic() {
        let ax = axis(500.0);
        let secs = [
            Section {
                from: 0.0,
                to: 100.0,
                kind: SectionKind::Open,
            },
            Section {
                from: 100.0,
                to: 400.0,
                kind: SectionKind::Bored,
            },
            Section {
                from: 400.0,
                to: 500.0,
                kind: SectionKind::Gallery { hill_right: false },
            },
        ];
        let a = tunnel_meshes(&ax, &secs, 9);
        let b = tunnel_meshes(&ax, &secs, 9);
        assert_eq!(a.tube.positions, b.tube.positions);
        assert_eq!(a.tube.colors, b.tube.colors);
        assert_eq!(a.lit.positions, b.lit.positions);
        assert_eq!(tunnel_texture_pixels(), tunnel_texture_pixels());
    }
}
