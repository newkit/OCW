//! The rider's face on the near model (spec 14 Phase 4, 4N *Faces*; the
//! user, 2026-11-28: "the beards, lips and eyes of riders do look not so
//! realistic"): one sculpted skin over the front of the head – forehead,
//! brow ridge, eye sockets, nose, cheeks, the mound of the lips, chin and
//! the underside of the jaw – instead of ellipsoids stuck on the skull;
//! eyes with a white, an iris in the rider's eye colour, a pupil, a glint,
//! lids and a lash line; lips with a cupid's bow lying in that skin; and
//! eyebrows, moustaches and beards as thin shells on the skin that carry
//! the hair's density per vertex, so the renderer thins them into the skin
//! at their edges (`ocw-render`'s `gear.wesl`, `KIND_HAIR`).
//!
//! **The skin** ([`Face`]) is a grid of rays from the head's axis: each
//! vertex lies where its ray leaves a smooth union of rounded forms (the
//! [`forms`]), blended towards the skull and the neck as drawn at the
//! grid's rim, where it ends just under them – so the head's ellipsoid,
//! the hair and the helmets fitted to it stay as they are and the face
//! has no open edge. Everything else is placed on this grid's surface
//! ([`Face::surface`], [`Face::on`]), so nothing floats or sinks.
//!
//! Vertex colours (`COLOR_0`): on skin, eyes and lips a shade the paint
//! colour is multiplied with (the lid's shadow on the eye, the pupil and
//! the iris' dark rim, the line between the lips); on `hair` and
//! `facial_hair` the hair's density (red = green = blue, 0 … 1).
//!
//! Everything here is a pure function of constants: the same bytes on
//! every machine.

use std::f32::consts::PI;
use std::sync::OnceLock;

use glam::{Quat, Vec3};

use crate::mesh::{Parts, Shape, ray_last_hit};
use crate::rider::{self, Detail};

/// One rounded form of the head: an ellipsoid joined to the others with
/// a fillet of `k` (m).
#[derive(Debug, Clone, Copy)]
struct Blob {
    c: Vec3,
    r: Vec3,
    rot: Quat,
    k: f32,
}

/// The polynomial smooth minimum: `min(a, b)` with a fillet of `k`.
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

impl Blob {
    /// About the distance from `p` to the ellipsoid (negative inside).
    fn distance(&self, p: Vec3) -> f32 {
        let q = self.rot.inverse() * (p - self.c);
        let k0 = (q / self.r).length();
        let k1 = (q / (self.r * self.r)).length();
        if k1 < 1e-9 {
            return -self.r.min_element();
        }
        k0 * (k0 - 1.0) / k1
    }
}

/// The measures of the face (m, from the skull's centre; x to the side,
/// y up, the face towards −z), for the model, the renderer's camera and
/// the tests. Proportions after the artists' canon and anthropometry
/// (spec 4N *Faces*, *Research*): the head five eyes wide, one eye between
/// the eyes, the nose's wings under the inner corners, the nose's base
/// half-way from the brows to the chin, the mouth a third of the way from
/// the nose to the chin, its corners under the irises' inner edges.
pub mod at {
    /// The pupils (x either side): 63 mm apart.
    pub const EYE: [f32; 2] = [0.0315, 0.005];
    /// The lid opening's half width and how far the upper and the lower
    /// lid's edges are from the pupil at its middle: 29.6 × 9.4 mm, the
    /// upper lid over the iris' top 1.6 mm, the lower at its bottom.
    pub const EYE_HALF_WIDTH: f32 = 0.0148;
    pub const LID_UP: f32 = 0.0042;
    pub const LID_DOWN: f32 = 0.0052;
    /// The iris' and the pupil's radii.
    pub const IRIS: f32 = 0.0058;
    pub const PUPIL: f32 = 0.0019;
    /// The eye's surface: the ball's radius as seen in the lid opening
    /// (flatter than the real 12 mm ball: the corners lie on the lids'
    /// tissue, not on the ball), the lids' thickness over it, how far its
    /// centre is towards the nose from the pupil (the outer corner lies
    /// deeper than the inner) and the front of the lids (z).
    pub const BALL: f32 = 0.0195;
    pub const LID: f32 = 0.0011;
    pub const BALL_IN: f32 = 0.0025;
    pub const LID_FRONT: f32 = -0.0896;
    /// The eyebrows: the arch's height at its inner end.
    pub const BROW_Y: f32 = 0.0265;
    /// Under the nose (the nose's base) and the nose's half width.
    pub const NOSE_BASE_Y: f32 = -0.035;
    pub const NOSE_HALF_WIDTH: f32 = 0.017;
    /// The line between the lips and the mouth's half width (51 mm).
    pub const MOUTH_Y: f32 = -0.056;
    pub const MOUTH_HALF_WIDTH: f32 = 0.0255;
    /// The red of the lips: the upper lip's height at the bow's peaks
    /// and the lower lip's at the middle (1 : 1.45).
    pub const LIP_UP: f32 = 0.0066;
    pub const LIP_DOWN: f32 = 0.0096;
    /// The chin's lowest point.
    pub const CHIN_Y: f32 = -0.097;
}

/// The forms the face is the smooth union of (from the skull's centre).
fn forms(s: Vec3) -> Vec<Blob> {
    let mut out = Vec::new();
    let mut add = |c: [f32; 3], r: [f32; 3], tilt: f32, k: f32, both: bool| {
        let sides: &[f32] = if both { &[-1.0, 1.0] } else { &[1.0] };
        for side in sides {
            out.push(Blob {
                c: s + Vec3::new(side * c[0], c[1], c[2]),
                r: Vec3::from(r),
                rot: Quat::from_rotation_x(tilt),
                k,
            });
        }
    };
    // The skull as it is (the hair and the helmets are fitted to it).
    add([0.0, 0.0, 0.0], rider::SKULL_RADII, 0.0, 0.01, false);
    // The middle of the face: the upper jaw and the cheeks.
    add(
        [0.0, -0.040, -0.028],
        [0.066, 0.060, 0.068],
        0.0,
        0.014,
        false,
    );
    // The lower jaw – the ellipsoid the helmets' straps and buckles are
    // fitted to ([`rider::face::JAW`]) – and the chin on it.
    let (c, r, tilt) = rider::face::JAW;
    add(c, r, tilt, 0.012, false);
    add(
        [0.0, -0.083, -0.083],
        [0.021, 0.0135, 0.015],
        0.0,
        0.010,
        false,
    );
    // The neck under the jaw (so the jaw's underside runs into it).
    add(
        [0.0, -0.115, -0.002],
        [0.043, 0.07, 0.043],
        0.0,
        0.008,
        false,
    );
    // The brow ridge and the cheekbones.
    add(
        [0.030, 0.027, -0.079],
        [0.022, 0.009, 0.014],
        0.0,
        0.008,
        true,
    );
    add(
        [0.0, 0.026, -0.086],
        [0.014, 0.010, 0.010],
        0.0,
        0.008,
        false,
    );
    add(
        [0.045, -0.012, -0.060],
        [0.019, 0.016, 0.017],
        0.0,
        0.016,
        true,
    );
    // The nose: the bridge down to the tip, the tip, the wings.
    add(
        [0.0, -0.007, -0.094],
        [0.0068, 0.020, 0.0085],
        0.49,
        0.006,
        false,
    );
    add(
        [0.0, -0.025, -0.1075],
        [0.0085, 0.0082, 0.0090],
        0.0,
        0.005,
        false,
    );
    add(
        [0.0118, -0.029, -0.097],
        [0.0068, 0.0062, 0.0090],
        0.0,
        0.004,
        true,
    );
    // The mound of the mouth.
    add(
        [0.0, -0.055, -0.083],
        [0.027, 0.017, 0.016],
        0.0,
        0.012,
        false,
    );
    out
}

/// The grid's rows as heights `h` (m from the skull's centre): above
/// [`PIVOT`] a row is the horizontal rays at that height; below, the rays
/// from the axis at [`PIVOT`] tilted down by `(PIVOT − h) / TURN` rad –
/// round the chin and under the jaw to the neck.
const ROWS: [f32; 24] = [
    0.052, 0.043, 0.035, 0.029, 0.023, 0.017, 0.011, 0.005, -0.001, -0.007, -0.014, -0.021, -0.028,
    -0.035, -0.043, -0.050, -0.056, -0.062, -0.070, -0.079, -0.089, -0.099, -0.109, -0.118,
];
/// The grid's columns: the angle round the head's axis (degrees, 0 the
/// middle of the face; mirrored).
const COLUMNS: [f32; 13] = [
    0.0, 3.0, 6.0, 9.5, 13.5, 18.0, 22.5, 27.0, 32.0, 38.0, 47.0, 60.0, 79.0,
];
const PIVOT: f32 = -0.070;
const TURN: f32 = 0.05;
/// The axis the rays start from (z from the skull's centre).
const AXIS_Z: f32 = -0.012;
/// The face's skin lies this far over the head's own where both are
/// drawn, and ends this far under it at its rim – a little at the top
/// and the sides (the skull is convex there; a deeper rim would show as
/// a fold in the light), more under the jaw, where the skull meets the
/// neck in a groove.
pub const OVER: f32 = 0.0004;
pub const RIM_UNDER: f32 = 0.0006;
pub const RIM_UNDER_JAW: f32 = 0.003;

/// The sculpted skin of the face: the grid's rays and how far each
/// reaches.
#[derive(Debug, Clone)]
pub struct Face {
    skull: Vec3,
    /// Column angles (rad, ascending) and row heights (descending).
    a: Vec<f32>,
    h: Vec<f32>,
    /// The reach of each vertex's ray (row-major: `r[j * a.len() + i]`).
    r: Vec<f32>,
}

/// The centres of the two eyes' surfaces and their sides (−1, 1).
fn eye_centres(s: Vec3) -> [(Vec3, f32); 2] {
    [-1.0f32, 1.0].map(|side| {
        (
            s + Vec3::new(
                side * (at::EYE[0] - at::BALL_IN),
                at::EYE[1],
                at::LID_FRONT + at::BALL + at::LID,
            ),
            side,
        )
    })
}

impl Face {
    fn ray(&self, a: f32, h: f32) -> (Vec3, Vec3) {
        let (y, pitch) = if h >= PIVOT {
            (h, 0.0)
        } else {
            (PIVOT, (h - PIVOT) / TURN)
        };
        (
            self.skull + Vec3::new(0.0, y, AXIS_Z),
            Vec3::new(a.sin() * pitch.cos(), pitch.sin(), -a.cos() * pitch.cos()),
        )
    }

    /// The shared face (built once per process).
    pub fn get() -> &'static Face {
        static FACE: OnceLock<Face> = OnceLock::new();
        FACE.get_or_init(Face::build)
    }

    fn build() -> Face {
        let skull = rider::skull_centre();
        let mut drawn = Shape::default();
        rider::skull_skin(&mut drawn, skull, Detail::High);
        rider::neck_skin(&mut drawn, Detail::High);
        let drawn: Vec<[Vec3; 3]> = drawn.tris().collect();
        let forms = forms(skull);
        let field = |p: Vec3| {
            forms
                .iter()
                .fold(f32::MAX, |d, b| smin(d, b.distance(p), b.k))
        };
        let a: Vec<f32> = COLUMNS[1..]
            .iter()
            .rev()
            .map(|d| -d.to_radians())
            .chain(COLUMNS.iter().map(|d| d.to_radians()))
            .collect();
        let h = ROWS.to_vec();
        let mut face = Face {
            skull,
            a,
            h,
            r: Vec::new(),
        };
        let (a_max, h_top, h_bot) = (
            face.a[face.a.len() - 1],
            face.h[0],
            face.h[face.h.len() - 1],
        );
        let eyes = eye_centres(skull);
        let mut r = Vec::with_capacity(face.a.len() * face.h.len());
        for (j, &h) in face.h.iter().enumerate() {
            for (i, &a) in face.a.iter().enumerate() {
                let (o, d) = face.ray(a, h);
                let skin = ray_last_hit(o, d, &drawn).unwrap_or(0.0);
                let rim = i == 0 || j == 0 || i + 1 == face.a.len() || j + 1 == face.h.len();
                if rim {
                    let under = if j + 1 == face.h.len() {
                        RIM_UNDER_JAW
                    } else {
                        RIM_UNDER
                    };
                    r.push(skin - under);
                    continue;
                }
                // Where the ray leaves the forms: from outside inwards.
                let mut t = 0.2;
                while t > 0.0 && field(o + d * t) > 0.0 {
                    t -= 0.002;
                }
                let (mut lo, mut hi) = (t.max(0.0), t + 0.002);
                for _ in 0..16 {
                    let m = (lo + hi) / 2.0;
                    if field(o + d * m) > 0.0 {
                        hi = m
                    } else {
                        lo = m
                    }
                }
                let mut form = (lo + hi) / 2.0;
                // The eye sockets: the skin stays behind the lids.
                for (c, _) in eyes {
                    let p = o + d * form;
                    let q = p - c;
                    // (Only in and just round the lid opening: the lids
                    // run out onto the skin above and below it.)
                    let off = Vec3::new(
                        (q.x - side_of(c, skull) * at::BALL_IN) / 0.0165,
                        q.y / 0.0075,
                        0.0,
                    )
                    .length();
                    let w = 1.0 - smooth((off - 0.85) / 0.5);
                    if w > 0.0 {
                        // Where the ray leaves the lids' sphere.
                        // (2 mm behind the eye's white.)
                        let rr = at::BALL - 0.002;
                        let b = d.dot(o - c);
                        let disc = b * b - ((o - c).length_squared() - rr * rr);
                        if disc > 0.0 {
                            let out = -b + disc.sqrt();
                            form += (out.min(form) - form) * w;
                        }
                    }
                }
                let over = skin + OVER;
                // Towards the temples the face ends earlier: the hair
                // lies on the head's own skin there.
                let side = a_max - 34f32.to_radians() * smooth(h / h_top);
                let edge = ((side - a.abs()) / 20f32.to_radians())
                    .min((h_top - h) / 0.02)
                    .min((h - h_bot) / 0.02);
                r.push(over + (form.max(over) - over) * smooth(edge));
            }
        }
        face.r = r;
        face
    }

    /// The point of the face's surface at column angle `a` (rad) and row
    /// height `h` – on the grid as drawn (bilinear between its vertices) –
    /// and the direction out of the head there (its ray).
    pub fn surface(&self, a: f32, h: f32) -> (Vec3, Vec3) {
        let cell = |v: &[f32], x: f32, up: bool| {
            let n = v.len();
            let i = (0..n - 1)
                .find(|&i| if up { x <= v[i + 1] } else { x >= v[i + 1] })
                .unwrap_or(n - 2);
            (i, ((x - v[i]) / (v[i + 1] - v[i])).clamp(0.0, 1.0))
        };
        let (i, u) = cell(&self.a, a, true);
        let (j, v) = cell(&self.h, h, false);
        let n = self.a.len();
        let r = |i: usize, j: usize| self.r[j * n + i];
        let top = r(i, j) + (r(i + 1, j) - r(i, j)) * u;
        let bottom = r(i, j + 1) + (r(i + 1, j + 1) - r(i, j + 1)) * u;
        let (o, d) = self.ray(a, h);
        (o + d * (top + (bottom - top) * v), d)
    }

    /// The point of the face's surface `x` to the side at height `y` (m
    /// from the skull's centre; above the chin, [`PIVOT`]) and the
    /// direction out of the head there.
    pub fn on(&self, x: f32, y: f32) -> (Vec3, Vec3) {
        self.surface(self.angle(x, y), y.max(PIVOT))
    }

    /// The column angle (rad) at which the surface is `x` to the side at
    /// height `y` (at least [`PIVOT`]).
    pub fn angle(&self, x: f32, y: f32) -> f32 {
        let y = y.max(PIVOT);
        let (mut lo, mut hi) = (self.a[0], self.a[self.a.len() - 1]);
        for _ in 0..28 {
            let m = (lo + hi) / 2.0;
            if self.surface(m, y).0.x - self.skull.x < x {
                lo = m
            } else {
                hi = m
            }
        }
        (lo + hi) / 2.0
    }

    /// The grid's vertex `(i, j)`: its point and its ray's direction.
    fn vertex(&self, i: usize, j: usize) -> (Vec3, Vec3) {
        let (o, d) = self.ray(self.a[i], self.h[j]);
        (o + d * self.r[j * self.a.len() + i], d)
    }

    fn size(&self) -> (usize, usize) {
        (self.a.len(), self.h.len())
    }

    /// The skull's centre (rest-pose model coordinates).
    pub fn skull(&self) -> Vec3 {
        self.skull
    }

    /// How far `p` lies out of the face's surface, along the grid's ray
    /// through it (m; negative: under the skin).
    pub fn over(&self, p: Vec3) -> f32 {
        let q = p - self.skull - Vec3::new(0.0, 0.0, AXIS_Z);
        let flat = (q.x * q.x + q.z * q.z).sqrt();
        let below = q.y - PIVOT;
        let (h, reach) = if below >= 0.0 {
            (q.y, flat)
        } else {
            (
                PIVOT + below.atan2(flat) * TURN,
                (flat * flat + below * below).sqrt(),
            )
        };
        let a = q.x.atan2(-q.z);
        let on = self.surface(a, h).0;
        reach - (on - self.ray(a, h).0).length()
    }
}

/// Which side of the head an eye's centre `c` is on (−1, 1).
fn side_of(c: Vec3, skull: Vec3) -> f32 {
    (c.x - skull.x).signum()
}

/// 0 below 0, 1 above 1, smooth between.
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn grey(v: f32) -> [f32; 4] {
    [v, v, v, 1.0]
}

/// A grid of `nx × ny` vertices as triangles facing `out(i, j)`; `keep`
/// says which cells are drawn. Vertices no cell uses are left out.
fn grid(
    s: &mut Shape,
    (nx, ny): (usize, usize),
    at: impl Fn(usize, usize) -> (Vec3, [f32; 4]),
    out: impl Fn(usize, usize) -> Vec3,
    keep: impl Fn(usize, usize) -> bool,
) {
    let mut index = vec![u32::MAX; nx * ny];
    let (mut verts, mut colours, mut tris) = (Vec::new(), Vec::new(), Vec::new());
    let mut facing = 0.0;
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            if !keep(i, j) {
                continue;
            }
            let mut v = [0u32; 4];
            for (k, (di, dj)) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
                let n = (j + dj) * nx + i + di;
                if index[n] == u32::MAX {
                    index[n] = verts.len() as u32;
                    let (p, c) = at(i + di, j + dj);
                    verts.push(p);
                    colours.push(c);
                }
                v[k] = index[n];
            }
            let p = |k: usize| verts[v[k] as usize];
            facing += (p(1) - p(0)).cross(p(2) - p(0)).dot(out(i, j));
            for t in [[v[0], v[1], v[2]], [v[1], v[3], v[2]]] {
                // Cells folded to nothing (a tapered end) are left out.
                let [a, b, c] = t.map(|k| verts[k as usize]);
                if (b - a).cross(c - a).length() < 1e-10 {
                    continue;
                }
                tris.push(t);
            }
        }
    }
    // One winding for the whole grid (a part that turns back on itself,
    // as the long beard's underside, keeps facing out of itself):
    // counter-clockwise seen from outside.
    if facing < 0.0 {
        for t in &mut tris {
            t.swap(1, 2);
        }
    }
    let from = s.positions.len();
    let mut part = Shape::default();
    part.add_smooth(&verts, &tris);
    part.smooth_by_angle();
    let base = from as u32;
    s.positions.extend(part.positions);
    s.normals.extend(part.normals);
    s.indices.extend(part.indices.into_iter().map(|i| base + i));
    s.colour_from(from, |k| colours[k]);
}

/// The face's skin (paint slot `skin`): the whole grid, shaded a little
/// in the eye sockets, under the nose and under the lower lip.
fn skin(f: &Face, s: &mut Shape) {
    let size = f.size();
    let eyes = eye_centres(f.skull);
    // No shade on the skin itself: on a grid this coarse a darkened
    // socket shows as blotches, and the lids beside it as light patches
    // (seen in the first renders); the light does the work.
    let _ = eyes;
    let shade = |_: Vec3| 1.0;
    grid(
        s,
        size,
        |i, j| {
            let p = f.vertex(i, j).0;
            (p, grey(shade(p)))
        },
        |i, j| f.vertex(i, j).1,
        |_, _| true,
    );
}

/// A point of an eye's surface: `px` towards the temple and `py` up from
/// the pupil (m), on the sphere of radius `r` round the eye's centre `c`.
fn ball(c: Vec3, side: f32, px: f32, py: f32, r: f32) -> Vec3 {
    let (mut dx, mut dy) = (px + at::BALL_IN, py);
    // Never past the sphere's side: its rim stays 16° short of it.
    let off = (dx * dx + dy * dy).sqrt();
    if off > 0.96 * r {
        (dx, dy) = (dx * 0.96 * r / off, dy * 0.96 * r / off);
    }
    let depth = (r * r - dx * dx - dy * dy).sqrt();
    Vec3::new(c.x + side * dx, c.y + dy, c.z - depth)
}

/// The lid edges at `t` (−1 the inner corner … 1 the outer): up and down
/// from the pupil's height.
fn lid_edges(t: f32) -> (f32, f32) {
    let open = (1.0 - t * t).max(0.0);
    (
        at::LID_UP * open.powf(0.55) * (1.0 - 0.2 * t),
        -at::LID_DOWN * open.powf(0.8) * (1.0 + 0.15 * t),
    )
}

/// The eyes (near model): the white (paint slot `eye_white`, shaded under
/// the upper lid and into the corners), the iris with its pupil (`iris`:
/// the rider's eye colour; the vertex colours make the pupil black, the
/// rim dark), a glint (`eye_glint`), the lids (`skin`, from their edges out
/// into the face's skin) and the lash line on the upper lid (`eye`).
fn eyes(f: &Face, p: &mut Parts) {
    const N: usize = 6;
    let t_of = |m: usize| -1.0 + 2.0 * m as f32 / N as f32;
    for (c, side) in eye_centres(f.skull) {
        let out = |_: usize, _: usize| Vec3::NEG_Z;
        let (rb, rl) = (at::BALL, at::BALL + at::LID);
        // The white: the lid opening and a little under the lids.
        grid(
            p.slot("eye_white"),
            (N + 1, 4),
            |m, k| {
                let t = t_of(m);
                let (up, down) = lid_edges(t * 0.98);
                let v = k as f32 / 3.0;
                let y = (up + 0.0006) + (down - up - 0.0012) * v;
                let corner = 1.0 - 0.3 * t.abs().powi(3);
                let lid = [0.62, 0.92, 1.0, 0.9][k];
                (
                    ball(c, side, t * (at::EYE_HALF_WIDTH + 0.0004), y, rb),
                    grey(lid * corner),
                )
            },
            out,
            |_, _| true,
        );
        // The iris and the pupil: rings round the pupil's axis (radius,
        // shade): the pupil black, the iris lighter towards its middle,
        // its rim dark; the top lies in the lid's shadow.
        const RINGS: [(f32, f32); 4] = [
            (at::PUPIL, 0.02),
            (at::PUPIL + 0.0005, 0.85),
            (0.0042, 1.0),
            (at::IRIS, 0.42),
        ];
        const AROUND: usize = 12;
        let mut verts = vec![ball(c, side, 0.0, 0.0, rb + 0.0003)];
        let mut shades = vec![grey(0.02)];
        for (r, v) in RINGS {
            for m in 0..AROUND {
                let phi = m as f32 / AROUND as f32 * 2.0 * PI;
                verts.push(ball(c, side, r * phi.cos(), r * phi.sin(), rb + 0.0003));
                shades.push(grey(v * (1.0 - 0.35 * smooth(phi.sin() * r / at::IRIS))));
            }
        }
        // Counter-clockwise seen from the front (−z) on either side.
        let face = |t: [usize; 3]| {
            let t = t.map(|k| k as u32);
            if side > 0.0 { [t[0], t[2], t[1]] } else { t }
        };
        let mut tris = Vec::new();
        for m in 0..AROUND {
            let n = (m + 1) % AROUND;
            tris.push(face([0, 1 + m, 1 + n]));
            for k in 0..RINGS.len() - 1 {
                let (i, o) = (1 + k * AROUND, 1 + (k + 1) * AROUND);
                tris.push(face([i + m, o + m, o + n]));
                tris.push(face([i + m, o + n, i + n]));
            }
        }
        let iris = p.slot("iris");
        let from = iris.positions.len();
        iris.add_smooth(&verts, &tris);
        iris.colour_from(from, |k| shades[k]);
        // The glint: up and to the rider's right in both eyes.
        let (gx, gy) = (-side * 0.0021, 0.0019);
        let mut fan = vec![ball(c, side, gx, gy, rb + 0.0006)];
        fan.extend((0..6).map(|k| {
            let phi = k as f32 / 6.0 * 2.0 * PI;
            ball(
                c,
                side,
                gx + 0.0009 * phi.cos(),
                gy + 0.0009 * phi.sin(),
                rb + 0.0006,
            )
        }));
        let tris: Vec<[u32; 3]> = (0..6u32)
            .map(|k| {
                let (a, b) = (1 + k, 1 + (k + 1) % 6);
                // Counter-clockwise seen from the front (−z) on either side.
                if side > 0.0 { [0, b, a] } else { [0, a, b] }
            })
            .collect();
        let glint = p.slot("eye_glint");
        let from = glint.positions.len();
        glint.add_smooth(&fan, &tris);
        // White vertex colours (no change): Blender gives every primitive
        // of a mesh with a colour attribute one, so a node is coloured in
        // all its parts or in none (the round trip, 15F).
        glint.colour_from(from, |_| [1.0; 4]);
        // The lids: from the edge out into the face's skin.
        for upper in [true, false] {
            grid(
                p.slot("skin"),
                (N + 1, 3),
                |m, k| {
                    let t = t_of(m);
                    let (up, down) = lid_edges(t);
                    let edge = if upper { up } else { down };
                    let reach = if upper { 0.0125 } else { -0.0115 };
                    let far = reach * (1.0 - (0.72 * t).powi(2)).sqrt();
                    let v = [0.0, 0.45, 1.0][k];
                    let px = t * (at::EYE_HALF_WIDTH + 0.004 * v);
                    let py = edge + (far - edge) * v;
                    let x = c.x + side * (px + at::BALL_IN) - f.skull.x;
                    let (skin, d) = f.on(x, c.y + py - f.skull.y);
                    let p = if k < 2 {
                        ball(c, side, px, py, rl)
                    } else {
                        skin - d * 0.0006
                    };
                    // The upper lid's fold, a little darker.
                    (p, grey(if upper && k == 1 { 0.86 } else { 1.0 }))
                },
                out,
                |_, _| true,
            );
        }
        // The lash line along the upper lid's edge, thinning inwards.
        grid(
            p.slot("eye"),
            (N + 1, 2),
            |m, k| {
                let t = t_of(m);
                let up = lid_edges(t).0;
                let thick = 0.0003 + 0.0011 * smooth((t + 1.0) / 0.9) * smooth((1.02 - t) / 0.25);
                let p = if k == 0 {
                    ball(c, side, t * at::EYE_HALF_WIDTH, up + thick, rl + 0.0003)
                } else {
                    ball(c, side, t * at::EYE_HALF_WIDTH, up - 0.0004, rl + 0.0009)
                };
                (p, grey(1.0))
            },
            out,
            |_, _| true,
        );
    }
}

/// The upper lip's top edge and the line between the lips at `t` (−1 …
/// 1 across the mouth), as heights (m from the skull's centre): a cupid's
/// bow, the line dipping at the middle and lifting a little at the
/// corners.
pub fn lip_lines(t: f32) -> (f32, f32, f32) {
    let taper = (1.0 - t.abs().powf(2.2)).max(0.0).powf(0.8).max(0.06);
    let bow = 1.0 - 0.24 * (-(t / 0.11).powi(2)).exp();
    let line = at::MOUTH_Y - 0.0007 * (-(t / 0.25).powi(2)).exp() + 0.0007 * t * t;
    let top = line + at::LIP_UP * bow * taper;
    let fall = (1.0 - t * t).max(0.0).powf(0.85).max(0.05);
    (top, line, line - at::LIP_DOWN * fall)
}

/// The lips (paint slot `lips`; near model): the upper lip and the dark
/// inside of the mouth (`eye`) for the [`rider::LIPS`] node, the lower lip
/// for [`rider::LIP_LOWER`]. Both lie in the face's skin at their edges
/// and swell out of it; the vertex colours darken the upper lip (it faces
/// down) and the line where the lips meet.
fn lips(f: &Face) -> (Parts, Parts) {
    const T: [f32; 13] = [
        -1.0, -0.82, -0.6, -0.38, -0.2, -0.09, 0.0, 0.09, 0.2, 0.38, 0.6, 0.82, 1.0,
    ];
    let out = |_: usize, _: usize| Vec3::NEG_Z;
    let on = |t: f32, y: f32, swell: f32| {
        let (p, d) = f.on(t * at::MOUTH_HALF_WIDTH, y);
        p + d * swell
    };
    let full = |t: f32| (1.0 - t.abs().powf(2.5)).max(0.0).powf(0.7);
    let (mut upper, mut lower) = (Parts::new(), Parts::new());
    grid(
        upper.slot("lips"),
        (T.len(), 4),
        |m, k| {
            let t = T[m];
            let (top, line, _) = lip_lines(t);
            let v = k as f32 / 3.0;
            // (At the corners the lip lies flat in the skin.)
            let swell = -0.0005 + [0.0, 0.0018, 0.0023, 0.0010][k] * full(t);
            let shade = [0.9, 0.86, 0.8, 0.5][k];
            (on(t, top + (line - top) * v, swell), grey(shade))
        },
        out,
        |_, _| true,
    );
    // The dark inside: on the skin behind the closed lower lip, shown
    // where it drops.
    grid(
        upper.slot("eye"),
        (7, 3),
        |m, k| {
            let t = (m as f32 / 3.0 - 1.0) * 0.62;
            let (_, line, _) = lip_lines(t);
            let deep = rider::face::LIP_DROP_M + 0.0008;
            let round = (1.0 - t * t / 0.62f32.powi(2)).max(0.04).sqrt();
            let y = line + 0.0002 - deep * (k as f32 / 2.0) * round;
            (on(t, y, 0.0001), grey(1.0))
        },
        out,
        |_, _| true,
    );
    grid(
        lower.slot("lips"),
        (T.len(), 5),
        |m, k| {
            let t = T[m];
            let (_, line, bottom) = lip_lines(t);
            let v = [0.0, 0.3, 0.68, 1.0, 1.0][k];
            // The last row turns back into the head: the lip keeps its
            // lower edge when it drops over the chin.
            let swell = -0.0005 + [0.0010, 0.0029, 0.0027, 0.0, -0.003][k] * full(t);
            let shade = [0.55, 0.98, 1.0, 0.92, 0.92][k];
            let y = line + (bottom - line) * v - if k == 4 { 0.0006 } else { 0.0 };
            (on(t, y, swell), grey(shade))
        },
        out,
        |_, _| true,
    );
    (upper, lower)
}

/// The eyebrows (paint slot `hair`, density in the vertex colours): an
/// arch on the brow ridge, thick at the inner end, thin at the tail.
fn brows(f: &Face, s: &mut Shape) {
    for side in [-1.0f32, 1.0] {
        grid(
            s,
            (8, 3),
            |m, k| {
                let t = m as f32 / 7.0;
                let x = side * (0.0105 + 0.041 * t);
                let arch = at::BROW_Y + 0.0045 * (PI * t.powf(0.8)).sin() - 0.0035 * t;
                let half = 0.0031 - 0.0019 * t;
                let (y, lift, dense) = match k {
                    0 => (arch + half, -0.0001, 0.0),
                    1 => (arch, 0.0009, 1.0),
                    _ => (arch - half, -0.0001, 0.0),
                };
                let (p, d) = f.on(x, y);
                let ends = smooth(t / 0.12) * smooth((1.0 - t) / 0.2);
                (p + d * lift, grey(dense * (0.35 + 0.65 * ends)))
            },
            |_, _| Vec3::NEG_Z,
            |_, _| true,
        );
    }
}

/// The nostrils: two dark patches (`eye`) on the underside of the nose.
fn nostrils(f: &Face, s: &mut Shape) {
    for side in [-1.0f32, 1.0] {
        let (c, d) = f.on(side * 0.0056, at::NOSE_BASE_Y + 0.0012);
        let across = Vec3::X * 0.0028;
        let up = (Vec3::Y - d * d.y).normalize_or(Vec3::Y) * 0.0019;
        let mut fan = vec![c + d * 0.0004];
        fan.extend((0..6).map(|k| {
            let phi = k as f32 / 6.0 * 2.0 * PI;
            c + d * 0.0003 + across * phi.cos() + up * phi.sin()
        }));
        let tris: Vec<[u32; 3]> = (0..6u32)
            .map(|k| {
                let (a, b) = (1 + k, 1 + (k + 1) % 6);
                let n = (fan[a as usize] - fan[0]).cross(fan[b as usize] - fan[0]);
                if n.dot(d) > 0.0 { [0, a, b] } else { [0, b, a] }
            })
            .collect();
        s.add_smooth(&fan, &tris);
    }
}

/// A thin band on the skin along `path` (points as `(x, y)` from the
/// skull's centre), `half` wide at its middle, pointed at its ends.
fn crease(f: &Face, s: &mut Shape, path: &[[f32; 2]], half: f32) {
    // Every stretch in two, so the band follows the skin between the
    // path's points.
    let mut fine = Vec::new();
    for w in path.windows(2) {
        fine.push(w[0]);
        fine.push([(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0]);
    }
    fine.push(path[path.len() - 1]);
    let path = if path.len() == 4 { &fine[..] } else { path };
    let pts: Vec<(Vec3, Vec3)> = path.iter().map(|q| f.on(q[0], q[1])).collect();
    let n = pts.len();
    grid(
        s,
        (n, 2),
        |m, k| {
            let (p, d) = pts[m];
            let along = pts[(m + 1).min(n - 1)].0 - pts[m.saturating_sub(1)].0;
            let across = d.cross(along).normalize_or(Vec3::X);
            let t = m as f32 / (n - 1) as f32;
            let w = half * (0.15 + 0.85 * (PI * t).sin());
            (
                p + d * 0.0003 + across * if k == 0 { w } else { -w },
                grey(1.0),
            )
        },
        |m, _| pts[m].1,
        |_, _| true,
    );
}

/// The face's lines with the age (paint slot `skin_line`): `deep` false
/// from the 40s (crow's feet, the folds from the nose to the mouth's
/// corners), true from the 60s (longer and wider, lines under the eyes
/// and on the forehead).
fn lines(f: &Face, deep: bool) -> Parts {
    let mut p = Parts::new();
    let s = p.slot("skin_line");
    let k = if deep { 1.0 } else { 0.0 };
    for side in [-1.0f32, 1.0] {
        let x = |v: f32| side * v;
        // From the nose's wing round the mouth's corner.
        crease(
            f,
            s,
            &[
                [x(0.0165), -0.028],
                [x(0.0215), -0.037],
                [x(0.0265), -0.047],
                [x(0.0295), -0.056 - 0.006 * k],
            ],
            0.0005 + 0.0003 * k,
        );
        // Crow's feet at the outer corners.
        let fan: &[f32] = if deep {
            &[0.35, 0.0, -0.35]
        } else {
            &[0.22, -0.22]
        };
        for slope in fan {
            let x0 = at::EYE[0] + at::EYE_HALF_WIDTH + 0.003;
            let len = 0.0065 + 0.002 * k;
            crease(
                f,
                s,
                &[
                    [x(x0), at::EYE[1] + 0.0015 * slope.signum()],
                    [x(x0 + len / 2.0), at::EYE[1] + slope * len / 2.0],
                    [x(x0 + len), at::EYE[1] + slope * len],
                ],
                0.00035 + 0.00015 * k,
            );
        }
        if deep {
            // Under the eye.
            crease(
                f,
                s,
                &[
                    [x(0.020), -0.0075],
                    [x(0.029), -0.0105],
                    [x(0.039), -0.0085],
                ],
                0.0005,
            );
        }
    }
    if deep {
        for y in [0.040, 0.048] {
            crease(
                f,
                s,
                &[
                    [-0.026, y],
                    [-0.013, y + 0.001],
                    [0.0, y + 0.0013],
                    [0.013, y + 0.001],
                    [0.026, y],
                ],
                0.00055,
            );
        }
    }
    p
}

/// The older riders' softer jaw (paint slot `skin`; from the 60s): the
/// skin hanging a little over the jaw's edge either side of the chin.
fn soft_jaw(f: &Face) -> Parts {
    let mut p = Parts::new();
    for side in [-1.0f32, 1.0] {
        let (c, d) = f.surface(side * 34f32.to_radians(), -0.084);
        p.slot("skin").ellipsoid(
            c - d * 0.0085,
            Vec3::new(0.012, 0.010, 0.014),
            Quat::IDENTITY,
            3,
            6,
        );
    }
    p
}

/// How a beard grows at a point of the face: its density (0 none … 1
/// full) and how far it stands out of the skin (m).
type Growth = fn(&Face, Vec3, f32, f32) -> (f32, f32);

/// The bare skin round the lips a beard leaves free (m), on every side
/// of both lips, the lower one dropped as far as when speaking.
pub const LIP_MARGIN: f32 = 0.0015;

/// How far `q` (from the skull's centre) is from the lips' red (m; 0 on
/// it), the lower lip dropped by `drop`.
fn from_lips(q: Vec3, drop: f32) -> f32 {
    let t = (q.x / at::MOUTH_HALF_WIDTH).clamp(-1.0, 1.0);
    let (top, _, bottom) = lip_lines(t);
    let dx = (q.x.abs() - at::MOUTH_HALF_WIDTH).max(0.0);
    let dy = (q.y - top).max(bottom - drop - q.y).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// 1 where a beard may grow round the mouth, 0 on the closed lips and
/// [`LIP_MARGIN`] round them.
fn off_lips(q: Vec3) -> f32 {
    smooth((from_lips(q, 0.0) - LIP_MARGIN) / 0.003)
}

/// How much of its length a beard has at `q`: short at the lips – also
/// where the lower lip drops to while speaking, so the lip stays in
/// front of it – and full [`LIP_CLEAR`] away.
fn by_lips(q: Vec3) -> f32 {
    smooth(from_lips(q, rider::face::LIP_DROP_M) / LIP_CLEAR)
}

/// A beard has its full length this far from the lips (m).
pub const LIP_CLEAR: f32 = 0.008;
/// At the lips a beard is at most this long (m): under the lips' swell.
pub const LIP_SHORT: f32 = 0.0004;

/// The cheek line: 1 below the line from the sideburn down to beside the
/// mouth's corner (lifted by `lift`), fading over `soft` above it.
fn below_cheek_line(q: Vec3, lift: f32, soft: f32) -> f32 {
    let ax = q.x.abs();
    let cheek = -0.041 + (ax - 0.027).max(0.0) * 0.72 + lift;
    // Under the nose the beard reaches up to the nose's base.
    let nose = at::NOSE_BASE_Y - 0.003;
    let w = smooth((ax - 0.019) / 0.01);
    smooth((nose + (cheek - nose) * w - q.y) / soft)
}

/// 1 above the neck line under the jaw (row height `to`), fading over
/// `soft` below it; and 1 inside the sideburns' angle.
fn above_neck_line(a: f32, h: f32, to: f32, soft: f32) -> f32 {
    smooth((h - to) / soft) * smooth((74f32.to_radians() - a.abs()) / 8f32.to_radians())
}

/// How much of the chin a point is (1 at its front and under it).
fn chin(q: Vec3) -> f32 {
    let d = Vec3::new(q.x / 0.03, (q.y + 0.087) / 0.03, 0.0).length();
    1.0 - smooth(d - 0.2)
}

fn stubble(_: &Face, q: Vec3, a: f32, h: f32) -> (f32, f32) {
    let d = below_cheek_line(q, 0.012, 0.014) * above_neck_line(a, h, -0.116, 0.012) * off_lips(q);
    (0.5 * d, 0.0005)
}

fn short_stubble(_: &Face, q: Vec3, a: f32, h: f32) -> (f32, f32) {
    let d = below_cheek_line(q, 0.009, 0.010) * above_neck_line(a, h, -0.114, 0.010) * off_lips(q);
    (0.82 * d, 0.0011)
}

fn short_boxed(_: &Face, q: Vec3, a: f32, h: f32) -> (f32, f32) {
    let d = below_cheek_line(q, 0.0, 0.004) * above_neck_line(a, h, -0.106, 0.005) * off_lips(q);
    (d, 0.0035 + 0.002 * chin(q))
}

fn full(_: &Face, q: Vec3, a: f32, h: f32) -> (f32, f32) {
    let d = below_cheek_line(q, 0.006, 0.012) * above_neck_line(a, h, -0.110, 0.008) * off_lips(q);
    (d, 0.006 + 0.005 * chin(q))
}

fn goatee(_: &Face, q: Vec3, a: f32, h: f32) -> (f32, f32) {
    let round = Vec3::new(q.x / 0.022, (q.y + 0.084) / 0.024, 0.0).length();
    let d =
        (1.0 - smooth((round - 0.8) / 0.25)) * above_neck_line(a, h, -0.104, 0.006) * off_lips(q);
    (d, 0.006)
}

fn chin_strap(_: &Face, _: Vec3, a: f32, h: f32) -> (f32, f32) {
    // Along the jaw's edge from under one ear to under the other.
    let u = a.abs() / 79f32.to_radians();
    let line = -0.089 + 0.029 * u * u;
    let d = 1.0 - smooth(((h - line).abs() - 0.0045) / 0.003);
    (
        d * smooth((76f32.to_radians() - a.abs()) / 6f32.to_radians()),
        0.003,
    )
}

/// How far the long beard hangs below a point of the chin's and the
/// jaw's front (m): the hair's own weight, not its length off the skin.
fn hang(q: Vec3, h: f32) -> f32 {
    // (Only from the chin's front: behind it the straps meet.)
    let _ = h;
    0.036
        * smooth((-0.064 - q.y) / 0.026)
        * (1.0 - smooth((q.x.abs() - 0.02) / 0.032))
        * smooth((-0.074 - q.z) / 0.016)
}

/// The helmets' straps run over the beards (`crate::helmet` checks it):
/// beside the face, where they come down in front of the ears, and under
/// the jaw, where they meet at the buckle, a beard is at most this long
/// (m) – pressed flat under them.
pub const UNDER_STRAPS: f32 = 0.0025;

/// The share of its length a beard keeps at column angle `a`, row `h`:
/// 1 on the front of the face, 0 where the straps lie.
fn off_straps(a: f32, h: f32) -> f32 {
    (1.0 - smooth((a.abs() - 28f32.to_radians()) / 22f32.to_radians()))
        * smooth((h + 0.104) / 0.012)
}

/// The beards of `ocw_core::character::BEARDS` after "None", in
/// [`rider::opt::BEARD`]'s order.
const BEARDS: [(Growth, bool); 7] = [
    (stubble, false),
    (short_stubble, false),
    (short_boxed, false),
    (full, false),
    (goatee, false),
    (chin_strap, false),
    (full, true),
];

/// A beard (paint slot `facial_hair`): a shell over the face's grid where
/// it grows, standing out of the skin by its length, at its edge in the
/// skin; the vertex colours carry the density.
fn beard(f: &Face, (grow, hangs): (Growth, bool)) -> Parts {
    let (nx, ny) = f.size();
    let growth: Vec<(f32, f32)> = (0..nx * ny)
        .map(|n| {
            let (i, j) = (n % nx, n / nx);
            let rim = i == 0 || j == 0 || i + 1 == nx || j + 1 == ny;
            let q = f.vertex(i, j).0 - f.skull;
            let (d, thick) = grow(f, q, f.a[i], f.h[j]);
            if rim || d < 0.03 {
                (0.0, 0.0)
            } else {
                (d, thick)
            }
        })
        .collect();
    let mut p = Parts::new();
    grid(
        p.slot("facial_hair"),
        (nx, ny),
        |i, j| {
            let (at, d) = f.vertex(i, j);
            let (dense, thick) = growth[j * nx + i];
            // A beard thins to nothing at its edge; bare vertices lie
            // just under the skin.
            let q = at - f.skull;
            let lift = if dense > 0.0 {
                let long = thick * smooth(dense * 2.0);
                let flat = long.min(UNDER_STRAPS);
                let long = flat + (long - flat) * off_straps(f.a[i], f.h[j]);
                // Thin hair lies nearly in the skin, so a beard's edge is
                // no step in the light.
                let short = 0.00015 + (LIP_SHORT - 0.00015) * smooth(dense * 3.0);
                short + (long - short).max(0.0) * by_lips(q)
            } else {
                -0.0001
            };
            let down = if hangs && dense > 0.0 {
                // (Down and a little back towards the chest.)
                Vec3::new(0.0, -1.0, 0.3) * hang(q, f.h[j]) * by_lips(q)
            } else {
                Vec3::ZERO
            };
            (at + d * lift + down, grey(dense))
        },
        |i, j| f.vertex(i, j).1,
        |i, j| {
            [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .any(|(di, dj)| growth[(j + dj) * nx + i + di].0 > 0.0)
        },
    );
    p
}

/// A moustache (paint slot `facial_hair`): a shell on the upper lip
/// between the nose's base and the lip's red, `style` an index into
/// [`rider::opt::MOUSTACHE`] (classic, chevron, handlebar, horseshoe,
/// pencil).
fn moustache(f: &Face, style: usize) -> Parts {
    // Half width, thickness, how far it hangs over the lip's red, and
    // how far up it reaches (a share of the way to the nose's base).
    let (half, thick, over, reach) = match style {
        0 => (0.027, 0.0030, 0.0005, 1.0),
        1 => (0.031, 0.0045, 0.0028, 1.0),
        2 => (0.040, 0.0030, 0.0005, 1.0),
        3 => (0.0295, 0.0032, 0.0008, 1.0),
        _ => (0.026, 0.0012, -0.0006, 0.32),
    };
    const COLS: usize = 13;
    let mut p = Parts::new();
    grid(
        p.slot("facial_hair"),
        (COLS, 4),
        |m, k| {
            let u = m as f32 / (COLS - 1) as f32 * 2.0 - 1.0;
            let x = u * half;
            let t = (x / at::MOUTH_HALF_WIDTH).clamp(-1.0, 1.0);
            let top_lip = lip_lines(t).0;
            let bottom = top_lip + 0.0006 - over;
            // Up to under the nose at the middle, lower towards the ends.
            let roof = at::NOSE_BASE_Y - 0.0015 - 0.006 * u * u;
            let mut top = bottom + (roof - bottom).max(0.0015) * reach;
            let mut low = bottom;
            if style == 2 {
                // The handlebar's ends sweep out and up.
                let end = smooth((u.abs() - 0.62) / 0.38);
                top += 0.009 * end * end;
                low += 0.0085 * end * end + 0.0012 * end;
            }
            let v = [0.0, 0.3, 0.72, 1.0][k];
            let y = top + (low - top) * v;
            // Bare and in the skin all round: its open edge is never seen.
            let ends = smooth((1.0 - u.abs()) / 0.22);
            let dense = [0.0, 1.0, 1.0, 0.0][k] * ends;
            let lift = if dense > 0.0 {
                [0.0, thick, thick * 0.9, 0.0][k] * (0.3 + 0.7 * ends)
            } else {
                -0.0004
            };
            let (at, d) = f.on(x, y);
            (at + d * lift, grey(dense))
        },
        |_, _| Vec3::NEG_Z,
        |_, _| true,
    );
    if style == 3 {
        // The horseshoe's bars down beside the mouth to the jaw.
        for side in [-1.0f32, 1.0] {
            grid(
                p.slot("facial_hair"),
                (4, 7),
                |m, k| {
                    let v = k as f32 / 6.0;
                    let y = -0.047 - 0.042 * v;
                    let x = side * (0.0245 + [0.0, 0.003, 0.0065, 0.0095][m] - 0.004 * v * v);
                    // Below the chin's height the bar keeps its angle
                    // round the jaw.
                    let (at, d) = f.surface(f.angle(x, y), y);
                    let inner = (m == 1 || m == 2) && k > 0 && k < 6;
                    let end = smooth((1.0 - v) / 0.3);
                    let dense = if inner { 0.3 + 0.7 * end } else { 0.0 };
                    let lift = if inner {
                        0.0030 * (0.3 + 0.7 * end)
                    } else {
                        -0.0004
                    };
                    (at + d * lift, grey(dense))
                },
                |_, _| Vec3::NEG_Z,
                |_, _| true,
            );
        }
    }
    p
}

/// The ears (paint slot `skin`): a shell tilted out of the head, behind
/// the glasses' temples, under the helmet's straps.
fn ears(f: &Face, s: &mut Shape) {
    let (c, r) = (rider::face::EAR, rider::face::EAR_RADII);
    for side in [-1.0f32, 1.0] {
        s.ellipsoid(
            f.skull + Vec3::new(side * c[0], c[1], c[2]),
            Vec3::from(r),
            Quat::from_rotation_y(-side * 0.28) * Quat::from_rotation_x(-0.2),
            4,
            6,
        );
    }
}

/// Every part of the near model's face, in model coordinates.
#[derive(Debug, Clone)]
pub struct Set {
    /// The [`rider::FACE`] node: the skin, the eyes, the lids, the brows,
    /// the ears, the nostrils.
    pub face: Parts,
    /// The [`rider::LIPS`] node (the upper lip, the mouth's inside) and
    /// the [`rider::LIP_LOWER`] node.
    pub lips: (Parts, Parts),
    /// [`rider::opt::FACE_LINES`] and [`rider::opt::FACE_JAW`].
    pub lines: [Parts; 2],
    pub soft_jaw: Parts,
    /// [`rider::opt::BEARD`] and [`rider::opt::MOUSTACHE`].
    pub beards: Vec<Parts>,
    pub moustaches: Vec<Parts>,
}

/// The face's parts (built once per process).
pub fn set() -> &'static Set {
    static SET: OnceLock<Set> = OnceLock::new();
    SET.get_or_init(|| {
        let f = Face::get();
        let mut face = Parts::new();
        skin(f, face.slot("skin"));
        ears(f, face.slot("skin"));
        eyes(f, &mut face);
        brows(f, face.slot("hair"));
        nostrils(f, face.slot("eye"));
        Set {
            face,
            lips: lips(f),
            lines: [lines(f, false), lines(f, true)],
            soft_jaw: soft_jaw(f),
            beards: BEARDS.iter().map(|g| beard(f, *g)).collect(),
            moustaches: (0..5).map(|k| moustache(f, k)).collect(),
        }
    })
}

/// The centre of the mouth on the face's skin (rest-pose model
/// coordinates): the origin of the [`rider::LIPS`] node.
pub fn mouth() -> Vec3 {
    Face::get().on(0.0, at::MOUTH_Y).0
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::rider_split::{BEARD_BUDGET, MOUSTACHE_BUDGET};

    fn shape<'a>(p: &'a Parts, slot: &str) -> &'a Shape {
        p.shapes()
            .find(|(s, _)| *s == slot)
            .unwrap_or_else(|| panic!("no {slot}"))
            .1
    }

    /// The vertices on an open edge of `s` (an edge of one triangle only).
    fn open_edge(s: &Shape) -> Vec<usize> {
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for t in s.indices.chunks(3) {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let mut out: Vec<usize> = edges
            .into_iter()
            .filter(|(_, n)| *n == 1)
            .flat_map(|((a, b), _)| [a as usize, b as usize])
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The same face every time, on every machine: built twice, bit for
    /// bit the same; every number finite.
    #[test]
    fn the_face_is_built_the_same_every_time() {
        let (a, b) = (Face::build(), Face::build());
        assert_eq!(a.r.len(), a.a.len() * a.h.len());
        assert!(a.r.iter().all(|r| r.is_finite() && *r > 0.02 && *r < 0.16));
        let bits = |f: &Face| f.r.iter().map(|r| r.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&a), bits(&b));
        let again = beard(&a, BEARDS[3]);
        let set = set();
        let (x, y) = (
            shape(&again, "facial_hair"),
            shape(&set.beards[3], "facial_hair"),
        );
        assert_eq!(x.positions, y.positions);
        assert_eq!(x.colors, y.colors);
        assert_eq!(x.indices, y.indices);
    }

    /// The proportions of a face (4N *Faces*, *Research*): the head about
    /// five eyes wide with about one eye between the eyes; the nose's
    /// wings under the inner corners; the nose's base half-way from the
    /// brows to the chin; the mouth a third of the way from the nose to
    /// the chin, its corners under the irises; the upper lid over the top
    /// of the iris, the lower at its bottom; the lower lip fuller.
    #[test]
    fn the_face_has_a_faces_proportions() {
        use at::*;
        let eye = 2.0 * EYE_HALF_WIDTH;
        let head = 2.0 * rider::SKULL_RADII[0];
        assert!((4.6..5.6).contains(&(head / eye)), "{}", head / eye);
        let between = 2.0 * (EYE[0] - EYE_HALF_WIDTH);
        assert!((0.9..1.25).contains(&(between / eye)), "{}", between / eye);
        assert!((0.058..0.068).contains(&(2.0 * EYE[0])));
        assert!((NOSE_HALF_WIDTH - (EYE[0] - EYE_HALF_WIDTH)).abs() < 0.002);
        let mid = (BROW_Y + CHIN_Y) / 2.0;
        assert!((NOSE_BASE_Y - mid).abs() < 0.003, "{NOSE_BASE_Y} vs {mid}");
        let third = (NOSE_BASE_Y - MOUTH_Y) / (NOSE_BASE_Y - CHIN_Y);
        assert!((0.3..0.4).contains(&third), "{third}");
        const { assert!(MOUTH_HALF_WIDTH > EYE[0] - IRIS - 0.001 && MOUTH_HALF_WIDTH < EYE[0]) };
        assert!((0.001..0.0025).contains(&(IRIS - LID_UP)));
        assert!((0.0..0.001).contains(&(IRIS - LID_DOWN)));
        assert!((1.3..1.65).contains(&(LIP_DOWN / LIP_UP)));
        const { assert!(PUPIL < IRIS / 2.0 && IRIS < LID_UP + LID_DOWN) };
        // The mouth and the eyes where the model's constants say.
        let mouth = mouth() - rider::skull_centre();
        assert!(
            mouth.distance(Vec3::from(rider::face::MOUTH)) < 1e-3,
            "{mouth}"
        );
        assert_eq!(rider::face::EYE[..2], EYE);
    }

    /// No holes, no fold, nothing fighting the head's own skin: the
    /// face's rim lies under the skull and the neck as drawn, every other
    /// vertex over them, and every triangle of the grid faces out of the
    /// head. And it is a face: the nose is its foremost part, well in
    /// front of the lips; the brows stand in front of the eyes, the eyes'
    /// fronts in front of the skin of their sockets.
    #[test]
    fn the_faces_skin_is_closed_onto_the_head() {
        let f = Face::get();
        let mut drawn = Shape::default();
        rider::skull_skin(&mut drawn, f.skull, Detail::High);
        rider::neck_skin(&mut drawn, Detail::High);
        let drawn: Vec<[Vec3; 3]> = drawn.tris().collect();
        let (nx, ny) = f.size();
        for j in 0..ny {
            for i in 0..nx {
                let (o, d) = f.ray(f.a[i], f.h[j]);
                let skin = ray_last_hit(o, d, &drawn).expect("the head");
                let over = f.r[j * nx + i] - skin;
                if i == 0 || j == 0 || i + 1 == nx || j + 1 == ny {
                    assert!((-RIM_UNDER_JAW - 1e-6..=-RIM_UNDER + 1e-6).contains(&over));
                } else {
                    assert!(over >= OVER - 1e-6, "({i}, {j}): {over}");
                    assert!(over < 0.045, "({i}, {j}): {over}");
                }
            }
        }
        for j in 0..ny - 1 {
            for i in 0..nx - 1 {
                let p = |di: usize, dj: usize| f.vertex(i + di, j + dj).0;
                let d = f.vertex(i, j).1 + f.vertex(i + 1, j + 1).1;
                for [a, b, c] in [[p(0, 0), p(1, 0), p(0, 1)], [p(1, 0), p(1, 1), p(0, 1)]] {
                    // (Columns run towards +x, rows down.)
                    let n = (b - a).cross(c - a);
                    assert!(n.dot(d) > 0.0, "cell ({i}, {j}) folds");
                }
            }
        }
        let z = |x: f32, y: f32| f.on(x, y).0.z - f.skull.z;
        let foremost = (0..40)
            .map(|k| z(0.0, at::BROW_Y - k as f32 * 0.003))
            .fold(f32::MAX, f32::min);
        let nose = z(0.0, at::NOSE_BASE_Y + 0.008);
        assert!(
            nose < foremost + 0.002,
            "the nose's tip: {nose} vs {foremost}"
        );
        assert!(nose < z(0.0, at::MOUTH_Y) - 0.008, "in front of the lips");
        assert!(nose < z(0.0, at::EYE[1]) - 0.012, "the bridge lies back");
        let brow = z(at::EYE[0], at::BROW_Y);
        assert!(
            brow < at::LID_FRONT - 0.002,
            "the brow over the eye: {brow}"
        );
        for side in [-1.0, 1.0] {
            let socket = z(side * at::EYE[0], at::EYE[1]);
            assert!(socket > at::LID_FRONT + 0.0008, "{socket}");
        }
    }

    /// The eyes: per eye a white, an iris with a black pupil at its
    /// middle and a dark rim, a glint in front of it and a lash line; the
    /// eye's surface stands out of the socket's skin everywhere in the lid
    /// opening, and the lids run from their edges into the skin (their
    /// outer vertices under it).
    #[test]
    fn the_eyes_have_whites_irises_pupils_and_lids() {
        let f = Face::get();
        let face = &set().face;
        let iris = shape(face, "iris");
        let per_eye = iris.positions.len() / 2;
        for (k, (c, side)) in eye_centres(f.skull).into_iter().enumerate() {
            let pts = &iris.positions[k * per_eye..(k + 1) * per_eye];
            let cols = &iris.colors[k * per_eye..(k + 1) * per_eye];
            let pupil = pts[0] - f.skull;
            assert!((pupil.x - side * at::EYE[0]).abs() < 1e-5, "{pupil}");
            assert!((pupil.y - at::EYE[1]).abs() < 1e-5, "{pupil}");
            assert!(
                (pupil.z - (at::LID_FRONT + at::LID)).abs() < 5e-4,
                "{pupil}"
            );
            assert!(cols[0][0] < 0.05, "a black pupil");
            assert!(cols.iter().any(|c| c[0] > 0.95), "the iris in its colour");
            let rim = cols[per_eye - 1][0];
            assert!((0.2..0.6).contains(&rim), "a dark rim: {rim}");
            for p in pts {
                let r = Vec3::new(p.x - pts[0].x, p.y - pts[0].y, 0.0).length();
                assert!(r < at::IRIS + 1e-5);
            }
            // The opening: the eye's surface out of the socket.
            for m in 0..=8 {
                let t = -0.9 + 1.8 * m as f32 / 8.0;
                let (up, down) = lid_edges(t);
                for v in [0.1, 0.5, 0.9] {
                    let y = down + (up - down) * v;
                    let p = ball(c, side, t * at::EYE_HALF_WIDTH, y, at::BALL);
                    let over = f.over(p);
                    assert!(over > 0.0003, "eye {side} at {t}, {v}: {over}");
                }
            }
        }
        let white = shape(face, "eye_white");
        assert!(white.colors.iter().all(|c| c[0] <= 1.0 && c[0] > 0.3));
        assert!(white.colors.iter().any(|c| c[0] < 0.7), "the lid's shadow");
        let glint = shape(face, "eye_glint");
        assert_eq!(glint.triangles(), 12);
        let front = |s: &Shape| s.positions.iter().map(|p| p.z).fold(f32::MAX, f32::min);
        assert!(front(glint) < front(iris), "the glint on the iris");
        assert!(shape(face, "eye").triangles() >= 24, "lash lines, nostrils");
        // The lids' skin joins the face: their rims lie in the skin.
        let skin = shape(face, "skin");
        let (nx, ny) = f.size();
        let rest = &skin.positions[nx * ny..];
        assert!(rest.iter().any(|p| f.over(*p) < -0.0003), "lid rims");
    }

    /// The lips: a cupid's bow (the upper lip's edge dips at the middle
    /// between two peaks), a fuller lower lip, the corners thin; both lips
    /// lie in the skin at their outer edges and swell out of it; where
    /// they meet they are darker; the dark inside lies on the skin behind
    /// the closed lower lip and reaches as far down as the lip drops.
    #[test]
    fn the_lips_have_a_bow_and_lie_in_the_skin() {
        let f = Face::get();
        let (top0, line0, bottom0) = lip_lines(0.0);
        let (peak, _, _) = lip_lines(0.2);
        assert!(peak > top0 + 0.0008, "the bow: {peak} vs {top0}");
        assert!(((line0 - bottom0) - at::LIP_DOWN).abs() < 1e-6);
        let (corner_top, corner_line, corner_bottom) = lip_lines(1.0);
        assert!(corner_top - corner_bottom < 0.0015 && corner_line > line0);
        let (upper, lower) = &set().lips;
        for (lip, swell) in [
            (shape(upper, "lips"), 0.0015),
            (shape(lower, "lips"), 0.002),
        ] {
            let over: Vec<f32> = lip.positions.iter().map(|p| f.over(*p)).collect();
            let most = over.iter().copied().fold(f32::MIN, f32::max);
            assert!((swell..0.0035).contains(&most), "{most}");
            for i in open_edge(lip) {
                assert!(over[i] < 0.0009, "an edge {} mm out", over[i] * 1e3);
            }
            assert!(lip.colors.iter().any(|c| c[0] < 0.6), "dark at the line");
            let wide = at::MOUTH_HALF_WIDTH + 1e-3;
            assert!(
                lip.positions
                    .iter()
                    .all(|p| (p.x - f.skull.x).abs() <= wide)
            );
        }
        let inside = shape(upper, "eye");
        let lowest = inside
            .positions
            .iter()
            .map(|p| p.y)
            .fold(f32::MAX, f32::min)
            - f.skull.y;
        assert!(lowest < line0 - rider::face::LIP_DROP_M, "{lowest}");
        for p in &inside.positions {
            let over = f.over(*p);
            assert!((0.0..0.0004).contains(&over), "{over}");
            let q = *p - f.skull;
            let t = q.x / at::MOUTH_HALF_WIDTH;
            let (_, line, bottom) = lip_lines(t);
            assert!(q.y < line + 0.0005 && q.y > bottom + 0.0005, "{q}");
        }
    }

    /// The beards and moustaches (near model): each is a shell whose open
    /// edges are bare (density 0) and lie in the skin – no hard edge, no
    /// floating shell –, whose hair stands out of the skin, never within
    /// [`LIP_MARGIN`] of the lips' red (moustaches may hang over the upper
    /// lip's edge) and at most [`LIP_SHORT`] long where the lower lip
    /// drops while speaking; mirrored left to right; in its budget; and
    /// each style is what its name says.
    #[test]
    fn the_facial_hair_thins_into_the_skin() {
        let f = Face::get();
        let set = set();
        let hairy = |p: &Parts| -> Vec<(Vec3, f32)> {
            let s = shape(p, "facial_hair");
            assert_eq!(s.colors.len(), s.positions.len());
            s.positions
                .iter()
                .map(|p| *p - f.skull)
                .zip(s.colors.iter().map(|c| c[0]))
                .collect()
        };
        for (k, (name, part)) in rider::opt::BEARD
            .iter()
            .zip(&set.beards)
            .chain(rider::opt::MOUSTACHE.iter().zip(&set.moustaches))
            .enumerate()
        {
            let beard = k < 7;
            let s = shape(part, "facial_hair");
            let budget = if beard {
                BEARD_BUDGET
            } else {
                MOUSTACHE_BUDGET
            };
            assert!(
                (60..=budget).contains(&s.triangles()),
                "{name}: {}",
                s.triangles()
            );
            let pts = hairy(part);
            assert!(pts.iter().all(|(_, d)| (0.0..=1.0).contains(d)), "{name}");
            for i in open_edge(s) {
                assert!(pts[i].1 == 0.0, "{name}: hair on an open edge");
                let over = f.over(s.positions[i]);
                assert!(over < 0.0, "{name}: an open edge {} mm out", over * 1e3);
            }
            for (q, d) in &pts {
                if *d == 0.0 {
                    continue;
                }
                let over = f.over(*q + f.skull);
                assert!(
                    over > 0.0001,
                    "{name}: hair {} mm in the skin at {q}",
                    -over * 1e3
                );
                assert!(over < 0.07, "{name}");
                if beard {
                    assert!(
                        from_lips(*q, 0.0) >= LIP_MARGIN,
                        "{name}: on the lips at {q}"
                    );
                    if from_lips(*q, rider::face::LIP_DROP_M) < 5e-4 {
                        assert!(over < LIP_SHORT + 2e-4, "{name}: {over} at the lips");
                    }
                } else if q.x.abs() < 0.012 {
                    assert!(q.y < at::NOSE_BASE_Y, "{name}: under the nose");
                }
                // Mirrored.
                let twin = pts.iter().any(|(o, e)| {
                    (o.x + q.x).abs() < 1e-5 && (o.y - q.y).abs() < 1e-5 && (e - d).abs() < 1e-4
                });
                assert!(twin, "{name}: no twin of {q}");
            }
        }
        let most = |k: usize| {
            hairy(&set.beards[k])
                .iter()
                .map(|p| p.1)
                .fold(0.0, f32::max)
        };
        assert!(
            (0.3..=0.55).contains(&most(0)),
            "stubble is thin: {}",
            most(0)
        );
        assert!((0.6..=0.85).contains(&most(1)), "{}", most(1));
        for k in 2..7 {
            assert!(most(k) > 0.99, "{}: {}", rider::opt::BEARD[k], most(k));
        }
        let dense = |k: usize| -> Vec<Vec3> {
            hairy(&set.beards[k])
                .into_iter()
                .filter(|p| p.1 > 0.5)
                .map(|p| p.0)
                .collect()
        };
        assert!(
            dense(4)
                .iter()
                .all(|q| q.x.abs() < 0.03 && q.y < at::MOUTH_Y),
            "the goatee on the chin"
        );
        assert!(
            dense(5)
                .iter()
                .all(|q| q.y < at::MOUTH_Y - 0.012 || q.x.abs() > 0.045),
            "the chin strap keeps to the jaw"
        );
        assert!(dense(5).iter().any(|q| q.x > 0.05) && dense(5).iter().any(|q| q.x.abs() < 0.01));
        let low = |k: usize| dense(k).iter().map(|q| q.y).fold(f32::MAX, f32::min);
        assert!(
            low(6) < low(3) - 0.012,
            "the long beard hangs: {} vs {}",
            low(6),
            low(3)
        );
        // The cheeks' upper part, the nose and the eyes stay bare.
        for k in 0..7 {
            assert!(dense(k).iter().all(|q| q.y < at::EYE[1] - 0.012), "{k}");
        }
    }

    /// Every part survives Blender (spec 15P): each triangle has an area
    /// and its vertex normals on its front; vertex colours, where a part
    /// has them, are never over 1.
    #[test]
    fn every_part_has_sound_triangles() {
        let set = set();
        let parts = [
            &set.face,
            &set.lips.0,
            &set.lips.1,
            &set.lines[0],
            &set.lines[1],
            &set.soft_jaw,
        ]
        .into_iter()
        .chain(&set.beards)
        .chain(&set.moustaches);
        for part in parts {
            for (slot, s) in part.shapes() {
                assert!(s.triangles() > 0, "{slot}");
                for t in s.indices.chunks(3) {
                    let [a, b, c] = [0, 1, 2].map(|k| s.positions[t[k] as usize]);
                    let n = (b - a).cross(c - a);
                    assert!(n.length() > 1e-10, "{slot}: a triangle without an area");
                    for k in t {
                        let v = s.normals[*k as usize];
                        assert!((v.length() - 1.0).abs() < 1e-3);
                        let at = s.positions[*k as usize] - Face::get().skull;
                        assert!(
                            v.dot(n) > 0.0,
                            "{slot}: a normal behind its triangle at {at}"
                        );
                    }
                }
                assert!(s.colors.len() <= s.positions.len());
                assert!(
                    s.colors.iter().flatten().all(|v| (0.0..=1.0).contains(v)),
                    "{slot}"
                );
            }
        }
    }
}
