//! The "newk it" bike and livery (spec 14 Phase 15, *The "newk it" bike*;
//! Phase 4, 4B): the user's own road bike, modelled from their photos.
//! The name "newk it" and the livery are the user's design and are used
//! with their permission; no other maker's name or logo is drawn (the
//! components, tyres, rims and computer are generic shapes).
//!
//! The livery is geometry in its own paint slots, no textures (WebGL 2 and
//! phones: no texture memory, sharp at any distance): "newk it" in hot
//! pink italic block letters with a light outline along both sides of the
//! down tube, a unicorn head on the head tube's front (white, closed eye,
//! rainbow mane, golden horn – our own drawing in the style of the user's
//! sticker), the rims' letters (pink at the front with a pink panel,
//! yellow at the rear) and the fork's pink inner faces. The frame's black
//! holographic flake with its fade to magenta is the renderer's `flake`
//! material (`ocw-render::gear`).
//!
//! Decals lie on the drawn surface: a tube is a prism of `sides` facets,
//! so every decal triangle is clipped at the facets' edges and laid flat
//! on its facet, a hair above it; a rim face is a plane.

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};

use crate::mesh::{Parts, Shape};

/// The bike's name in the garage (the user's brand, used with permission).
pub const NAME: &str = "newk it";
/// The frameset variant: the aero frame's shape tuned to the photos and
/// the bike's stock build, with the livery.
pub const FRAME_VARIANT: &str = "frame-aero-newkit";
/// The livery alone, a free paint for the other road frames.
pub const PAINT_VARIANT: &str = "paint-newkit";
/// Not a catalogue variant: the "newk it" frame painted another colour
/// (the livery off, the rest of its build kept).
pub const PAINT_PLAIN: &str = "paint-plain";
/// The file of the "newk it" bike.
pub const FILE: &str = "bikes/bike-road-aero-newkit.glb";

/// The livery's slots.
pub const FLAKE: &str = "flake";
pub const PINK: &str = "livery_pink";
pub const LIGHT: &str = "livery_light";
pub const YELLOW: &str = "livery_yellow";
pub const BLUE: &str = "livery_blue";
pub const GREEN: &str = "livery_green";
pub const WHITE: &str = "livery_white";
pub const INK: &str = "livery_ink";
pub const ANODISED: &str = "anodised_green";
pub const TAPE_PINK: &str = "bar_tape_pink";
pub const BOTTLE_BLACK: &str = "bottle_black";

/// The variants a bike is drawn with for frame colour `frame` (the look's
/// frame colour, `None`: unknown): the livery when the colour is
/// [`ocw_core::bike::NEWKIT_LIVERY`] (spec 14 Phase 4, 4B: a free paint
/// that travels as the frame colour, like bare carbon), the "newk it"
/// frame without its livery when it is painted another colour.
pub fn with_paint(mut variants: Vec<String>, frame: Option<ocw_core::Rgb>) -> Vec<String> {
    let Some(frame) = frame else {
        return variants;
    };
    if frame == ocw_core::bike::NEWKIT_LIVERY {
        if !variants.iter().any(|v| v == PAINT_VARIANT) {
            variants.push(PAINT_VARIANT.into());
        }
    } else if variants.iter().any(|v| v == FRAME_VARIANT) {
        variants.push(PAINT_PLAIN.into());
    }
    variants
}

/// The paint a new bike gets with a frameset of model `model`: the
/// "newk it" frame comes in its livery.
pub fn default_paint(model: &str) -> Option<ocw_core::Rgb> {
    (model == FRAME_VARIANT).then_some(ocw_core::bike::NEWKIT_LIVERY)
}

// --- The lettering -----------------------------------------------------------

/// A glyph of the retro rounded lower-case: its advance width and centre
/// lines in x-heights (baseline 0, x-height 1; ascenders to 1.45).
struct Glyph {
    ch: char,
    width: f32,
    strokes: &'static [&'static [(f32, f32)]],
}

const GLYPHS: &[Glyph] = &[
    Glyph {
        ch: 'n',
        width: 1.0,
        strokes: &[
            &[(0.0, 0.0), (0.0, 1.0)],
            &[(0.0, 1.0), (0.68, 1.0), (1.0, 0.68), (1.0, 0.0)],
        ],
    },
    Glyph {
        ch: 'e',
        width: 1.0,
        strokes: &[&[
            (0.0, 0.5),
            (1.0, 0.5),
            (1.0, 0.68),
            (0.68, 1.0),
            (0.32, 1.0),
            (0.0, 0.68),
            (0.0, 0.32),
            (0.32, 0.0),
            (1.0, 0.0),
        ]],
    },
    Glyph {
        ch: 'w',
        width: 1.5,
        strokes: &[
            &[
                (0.0, 1.0),
                (0.0, 0.32),
                (0.32, 0.0),
                (1.18, 0.0),
                (1.5, 0.32),
                (1.5, 1.0),
            ],
            &[(0.75, 0.0), (0.75, 0.78)],
        ],
    },
    Glyph {
        ch: 'k',
        width: 0.95,
        strokes: &[
            &[(0.0, 0.0), (0.0, 1.45)],
            &[(0.92, 1.0), (0.08, 0.42)],
            &[(0.36, 0.62), (0.95, 0.0)],
        ],
    },
    Glyph {
        ch: 'i',
        width: 0.0,
        strokes: &[&[(0.0, 0.0), (0.0, 1.0)], &[(0.0, 1.3), (0.0, 1.42)]],
    },
    Glyph {
        ch: 't',
        width: 0.9,
        strokes: &[
            &[(0.25, 1.42), (0.25, 0.32), (0.57, 0.0), (0.9, 0.0)],
            &[(0.0, 1.0), (0.82, 1.0)],
        ],
    },
];

/// The gap between letters and the width of a space (x-heights).
const GAP: f32 = 0.3;
const SPACE: f32 = 0.55;

/// The centre lines of `text` in x-heights, the left edge of the first
/// stroke's ink at 0 (`stroke`: the stroke width; `slant`: the italic's
/// shear). Returns the strokes and the ink's width.
pub fn layout(text: &str, stroke: f32, slant: f32) -> (Vec<Vec<Vec2>>, f32) {
    let mut out = Vec::new();
    let mut x = stroke * 0.5;
    let mut right: f32 = 0.0;
    for c in text.chars() {
        if c == ' ' {
            x += SPACE;
            continue;
        }
        let g = GLYPHS
            .iter()
            .find(|g| g.ch == c)
            .unwrap_or_else(|| panic!("no glyph {c:?}"));
        for s in g.strokes {
            out.push(
                s.iter()
                    .map(|&(u, v)| Vec2::new(x + u + slant * v, v))
                    .collect(),
            );
        }
        right = x + g.width + stroke * 0.5;
        x += g.width + stroke + GAP;
    }
    (out, right)
}

/// How a stroke ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ends {
    /// A round cap (the down tube's rounded letters).
    Round,
    /// Square, the stroke extended by half its width (the rims' blocks).
    Square,
}

/// Flat triangles in decal coordinates (metres on the surface; CCW).
#[derive(Debug, Clone, Default)]
pub struct Flat {
    pub pts: Vec<Vec2>,
    pub tris: Vec<[u32; 3]>,
}

impl Flat {
    fn add(&mut self, pts: &[Vec2], tris: &[[u32; 3]]) {
        let base = self.pts.len() as u32;
        self.pts.extend_from_slice(pts);
        self.tris.extend(tris.iter().map(|t| t.map(|i| base + i)));
    }

    pub fn triangles(&self) -> usize {
        self.tris.len()
    }

    /// A filled disc (`sides` triangles).
    pub fn disc(&mut self, c: Vec2, r: f32, sides: u32) {
        self.ellipse(c, Vec2::new(r, r), 0.0, sides);
    }

    /// A filled ellipse with half-axes `r`, turned by `rot` (radians).
    pub fn ellipse(&mut self, c: Vec2, r: Vec2, rot: f32, sides: u32) {
        let (s, co) = rot.sin_cos();
        let mut pts = vec![c];
        for k in 0..sides {
            let t = k as f32 / sides as f32 * TAU;
            let p = Vec2::new(r.x * t.cos(), r.y * t.sin());
            pts.push(c + Vec2::new(p.x * co - p.y * s, p.x * s + p.y * co));
        }
        let tris: Vec<[u32; 3]> = (0..sides)
            .map(|k| [0, 1 + k, 1 + (k + 1) % sides])
            .collect();
        self.add(&pts, &tris);
    }

    /// A filled convex polygon (any winding).
    pub fn polygon(&mut self, pts: &[Vec2]) {
        let area: f32 = (0..pts.len())
            .map(|i| pts[i].perp_dot(pts[(i + 1) % pts.len()]))
            .sum();
        let pts: Vec<Vec2> = if area < 0.0 {
            pts.iter().rev().copied().collect()
        } else {
            pts.to_vec()
        };
        let tris: Vec<[u32; 3]> = (1..pts.len() as u32 - 1).map(|k| [0, k, k + 1]).collect();
        self.add(&pts, &tris);
    }

    /// A stroke `2 hw` wide along `path` (mitred corners), cut into
    /// pieces at most `step` long (curved surfaces).
    pub fn stroke(&mut self, path: &[Vec2], hw: f32, ends: Ends, step: f32) {
        let mut path: Vec<Vec2> = path.to_vec();
        path.dedup_by(|a, b| a.distance(*b) < 1e-7);
        if path.len() < 2 {
            if let Some(&p) = path.first() {
                self.disc(p, hw, 8);
            }
            return;
        }
        let n = path.len();
        if ends == Ends::Square {
            let d0 = (path[1] - path[0]).normalize();
            let d1 = (path[n - 1] - path[n - 2]).normalize();
            path[0] -= d0 * hw;
            path[n - 1] += d1 * hw;
        }
        let dir = |i: usize| (path[i + 1] - path[i]).normalize();
        let offset = |i: usize| -> Vec2 {
            if i == 0 {
                return dir(0).perp() * hw;
            }
            if i == n - 1 {
                return dir(n - 2).perp() * hw;
            }
            let (a, b) = (dir(i - 1).perp(), dir(i).perp());
            let m = (a + b).normalize_or(a);
            m * (hw / m.dot(a).max(0.35))
        };
        for i in 0..n - 1 {
            let (a, b) = (path[i], path[i + 1]);
            let (oa, ob) = (offset(i), offset(i + 1));
            let k = ((a.distance(b) / step).ceil() as u32).max(1);
            let mut pts = Vec::with_capacity(2 * (k as usize + 1));
            for j in 0..=k {
                let t = j as f32 / k as f32;
                let c = a.lerp(b, t);
                let o = oa.lerp(ob, t);
                pts.push(c + o);
                pts.push(c - o);
            }
            let tris: Vec<[u32; 3]> = (0..k)
                .flat_map(|j| {
                    let (l0, r0, l1, r1) = (2 * j, 2 * j + 1, 2 * j + 2, 2 * j + 3);
                    [[l0, r0, l1], [r0, r1, l1]]
                })
                .collect();
            self.add(&pts, &tris);
        }
        if ends == Ends::Round {
            self.disc(path[0], hw, 8);
            self.disc(path[n - 1], hw, 8);
        }
    }

    /// The triangles cut at the line `p[axis] = at`: none crosses it.
    pub fn split(&mut self, axis: usize, at: f32) {
        let mut out = Flat::default();
        for t in &self.tris {
            let tri = t.map(|i| self.pts[i as usize]);
            let side = |p: Vec2| p[axis] - at;
            let s = tri.map(side);
            if s.iter().all(|v| *v >= -1e-7) || s.iter().all(|v| *v <= 1e-7) {
                out.add(&tri, &[[0, 1, 2]]);
                continue;
            }
            for keep in [1.0f32, -1.0] {
                let mut poly: Vec<Vec2> = Vec::with_capacity(4);
                for i in 0..3 {
                    let (p, q) = (tri[i], tri[(i + 1) % 3]);
                    let (sp, sq) = (s[i] * keep, s[(i + 1) % 3] * keep);
                    if sp >= 0.0 {
                        poly.push(p);
                    }
                    if (sp > 0.0 && sq < 0.0) || (sp < 0.0 && sq > 0.0) {
                        poly.push(p.lerp(q, sp / (sp - sq)));
                    }
                }
                for k in 1..poly.len().saturating_sub(1) {
                    let (a, b, c) = (poly[0], poly[k], poly[k + 1]);
                    if (b - a).perp_dot(c - a).abs() > 1e-12 {
                        out.add(&[a, b, c], &[[0, 1, 2]]);
                    }
                }
            }
        }
        *self = out;
    }

    /// Moved by `d` and scaled by `s` about the origin.
    pub fn transformed(&self, s: f32, d: Vec2) -> Flat {
        Flat {
            pts: self.pts.iter().map(|p| *p * s + d).collect(),
            tris: self.tris.clone(),
        }
    }
}

/// `text` as strokes: x-height `xh` (m), stroke width `stroke` and
/// outline `outline` (x-heights; 0: none), italic `slant`, centred on
/// the origin (the x-height's middle on v = 0). Returns (fill, outline).
pub fn text(
    s: &str,
    xh: f32,
    stroke: f32,
    outline: f32,
    slant: f32,
    ends: Ends,
    step: f32,
) -> (Flat, Flat) {
    let (strokes, width) = layout(s, stroke, slant);
    let centre = Vec2::new(width * 0.5, 0.62);
    let mut fill = Flat::default();
    let mut line = Flat::default();
    for st in &strokes {
        let pts: Vec<Vec2> = st.iter().map(|p| (*p - centre) * xh).collect();
        fill.stroke(&pts, stroke * 0.5 * xh, ends, step);
        if outline > 0.0 {
            line.stroke(&pts, (stroke * 0.5 + outline) * xh, ends, step);
        }
    }
    (fill, line)
}

// --- Surfaces ----------------------------------------------------------------

/// The drawn cross-section of a frame tube: a polygon of `n` corners on
/// the ellipse with half-axes `a` along `e1` and `b` along `e2` (the
/// vertices `a cos θ e1 + b sin θ e2`, θ = 2πk/n – `Shape::loft`).
#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub e1: Vec3,
    pub e2: Vec3,
    pub a: f32,
    pub b: f32,
    pub n: u32,
}

impl Section {
    /// The section of `frame_tube(a → b, r, aero, sides)` (bikes.rs): an
    /// ellipse with `r` across the bike (X) and `r·aero` in its plane, or
    /// a circle (its loft's own start direction).
    pub fn of_tube(axis: Vec3, r: f32, aero: f32, sides: u32) -> Self {
        let d = axis.normalize();
        if (aero - 1.0).abs() < 0.05 {
            let u = d.any_orthonormal_vector();
            Self {
                e1: u,
                e2: d.cross(u),
                a: r,
                b: r,
                n: sides,
            }
        } else {
            let u = (Vec3::X - d * Vec3::X.dot(d)).normalize();
            Self {
                e1: u,
                e2: d.cross(u),
                a: r,
                b: r * aero,
                n: sides,
            }
        }
    }

    fn corner(&self, k: u32) -> Vec2 {
        let t = (k % self.n) as f32 / self.n as f32 * TAU;
        Vec2::new(self.a * t.cos(), self.b * t.sin())
    }

    fn to3(&self, p: Vec2) -> Vec3 {
        self.e1 * p.x + self.e2 * p.y
    }

    fn to2(&self, v: Vec3) -> Vec2 {
        Vec2::new(v.dot(self.e1), v.dot(self.e2))
    }

    /// The point of the section's outline on the side `side` (a direction
    /// across the tube) whose coordinate along `across` (⟂ `side`) is
    /// `c`, and its facet's outward normal (both 3D, from the axis).
    pub fn at(&self, side: Vec3, across: Vec3, c: f32) -> (Vec3, Vec3) {
        let (s2, a2) = (self.to2(side).normalize(), self.to2(across).normalize());
        // Within the outline's extent on that side.
        let (lo, hi) = (0..self.n)
            .map(|k| self.corner(k))
            .filter(|p| p.dot(s2) > -1e-6)
            .fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                (lo.min(p.dot(a2)), hi.max(p.dot(a2)))
            });
        let c = c.clamp(lo, hi);
        let mut best: Option<(f32, Vec2, Vec2)> = None;
        for k in 0..self.n {
            let (p, q) = (self.corner(k), self.corner(k + 1));
            let (cp, cq) = (p.dot(a2), q.dot(a2));
            let (lo, hi) = (cp.min(cq), cp.max(cq));
            if c < lo - 1e-7 || c > hi + 1e-7 || (hi - lo) < 1e-9 {
                continue;
            }
            let mid = (p + q) * 0.5;
            let facing = mid.normalize().dot(s2);
            if facing <= 0.0 || best.is_some_and(|(f, _, _)| f >= facing) {
                continue;
            }
            let t = ((c - cp) / (cq - cp)).clamp(0.0, 1.0);
            let e = q - p;
            let mut nrm = Vec2::new(e.y, -e.x).normalize();
            if nrm.dot(mid) < 0.0 {
                nrm = -nrm;
            }
            best = Some((facing, p.lerp(q, t), nrm));
        }
        let (_, p, nrm) = best.expect("a facet on that side");
        (self.to3(p), self.to3(nrm).normalize())
    }

    /// The across-coordinates of the corners on the side `side` (where a
    /// decal must be cut so that each piece lies on one facet).
    pub fn creases(&self, side: Vec3, across: Vec3) -> Vec<f32> {
        let (s2, a2) = (self.to2(side).normalize(), self.to2(across).normalize());
        (0..self.n)
            .map(|k| self.corner(k))
            .filter(|p| p.dot(s2) > -1e-6)
            .map(|p| p.dot(a2))
            .collect()
    }
}

/// A decal on the side of a straight tube: decal u runs along `along`
/// (a unit vector along the tube), v along `across`; the origin is
/// `origin` on the axis.
pub struct TubeDecal {
    pub origin: Vec3,
    pub along: Vec3,
    pub side: Vec3,
    pub across: Vec3,
    pub section: Section,
}

impl TubeDecal {
    fn point(&self, p: Vec2) -> (Vec3, Vec3) {
        let (q, n) = self.section.at(self.side, self.across, p.y);
        (self.origin + self.along * p.x + q, n)
    }

    /// Lays `flat` on the tube, `lift` above it, into `shape`.
    pub fn emit(&self, shape: &mut Shape, flat: &Flat, lift: f32) {
        let mut f = flat.clone();
        for c in self.section.creases(self.side, self.across) {
            f.split(1, c);
        }
        emit_with(shape, &f, lift, |p| self.point(p));
    }
}

/// A decal on a rim's face (the plane x = `x` about `hub`): decal u runs
/// round the rim at radius `radius` (reading left to right seen from
/// that side), v outwards; the origin at angle `at` (0: straight up,
/// positive towards the rear, +Z).
pub struct RimDecal {
    pub hub: Vec3,
    pub x: f32,
    pub radius: f32,
    pub at: f32,
}

impl RimDecal {
    fn point(&self, p: Vec2) -> (Vec3, Vec3) {
        let side = self.x.signum();
        // Seen from +X the reader's right is the front (−Z): the angle
        // falls along u; from −X it grows.
        let phi = self.at - side * p.x / self.radius;
        let r = self.radius + p.y;
        let q = self.hub + Vec3::new(self.x, r * phi.cos(), r * phi.sin());
        (q, Vec3::X * side)
    }

    pub fn emit(&self, shape: &mut Shape, flat: &Flat, lift: f32) {
        emit_with(shape, flat, lift, |p| self.point(p));
    }
}

/// Maps `flat` through `map` (decal point → surface point and facet
/// normal) into `shape`, each triangle flat on its facet `lift` above
/// it, its winding facing out.
fn emit_with(shape: &mut Shape, flat: &Flat, lift: f32, map: impl Fn(Vec2) -> (Vec3, Vec3)) {
    for t in &flat.tris {
        let p = t.map(|i| flat.pts[i as usize]);
        if (p[1] - p[0]).perp_dot(p[2] - p[0]).abs() < 1e-12 {
            continue;
        }
        let centroid = (p[0] + p[1] + p[2]) / 3.0;
        let (_, n) = map(centroid);
        let mut q = p.map(|v| map(v).0 + n * lift);
        let face = (q[1] - q[0]).cross(q[2] - q[0]);
        if face.length_squared() < 1e-16 {
            continue;
        }
        if face.dot(n) < 0.0 {
            q.swap(1, 2);
        }
        let base = shape.positions.len() as u32;
        shape.positions.extend(q);
        shape.normals.extend([n; 3]);
        shape.indices.extend([base, base + 1, base + 2]);
    }
}

// --- The pieces of the livery ------------------------------------------------

/// The decal lifts above the surface (m): outline, fill, details.
const LIFT_OUTLINE: f32 = 0.0004;
const LIFT_FILL: f32 = 0.0007;
const LIFT_DETAIL: f32 = 0.001;

/// "newk it" along both sides of the down tube (from `a`, the bottom
/// bracket end, to `b`), on a tube of radius `r` and depth ratio `aero`
/// drawn with `sides` facets: hot pink, a light outline, italic, reading
/// from the rear to the front seen from either side (the user's bike:
/// from the head tube down on the left, up on the right), the tops of
/// the letters towards the top tube.
pub fn down_tube_text(p: &mut Parts, a: Vec3, b: Vec3, r: f32, aero: f32, sides: u32) {
    let section = Section::of_tube(b - a, r, aero, sides);
    let along = (b - a).normalize();
    // The letters as high as the tube's side allows (k's ascender
    // included): 26 mm on the aero tube.
    let xh = (0.8 * section.b).min(0.026);
    let (fill, line) = text(NAME, xh, 0.3, 0.075, 0.22, Ends::Round, 1.0);
    // Up: the side of the section towards the top tube.
    let up0 = (Vec3::Y - along * Vec3::Y.dot(along)).normalize();
    let origin = a.lerp(b, 0.56);
    for s in [-1.0f32, 1.0] {
        let side = Vec3::X * s;
        let decal = TubeDecal {
            origin,
            // From +X the reader's right is the front (−Z): up the tube.
            along: along * s,
            side,
            across: up0,
            section,
        };
        decal.emit(p.slot(LIGHT), &line, LIFT_OUTLINE);
        decal.emit(p.slot(PINK), &fill, LIFT_FILL);
    }
}

/// The unicorn head on the front of the head tube (from `bottom` to
/// `top`, radius `r`, depth ratio `aero`, `sides` facets): our own
/// drawing in the style of the user's sticker – a white head facing
/// left with a closed eye, a golden horn, a rainbow mane (pink, yellow,
/// blue, green) curling round the right and under the chin.
pub fn unicorn(p: &mut Parts, bottom: Vec3, top: Vec3, r: f32, aero: f32, sides: u32) {
    let section = Section::of_tube(top - bottom, r, aero, sides);
    let up = (top - bottom).normalize();
    // The tube's front: forward (−Z) square to its axis.
    let front = (Vec3::NEG_Z - up * Vec3::NEG_Z.dot(up)).normalize();
    // Seen from the front the viewer's right is the bike's left (−X).
    let right = (Vec3::NEG_X - up * Vec3::NEG_X.dot(up)).normalize();
    // The drawing's unit: about 7 mm on the aero head tube.
    let unit = (0.34 * section.a).min(0.0072);
    let centre = bottom.lerp(top, 0.55);
    // A decal on the front: u to the viewer's right, v up the tube.
    let decal = TubeDecalFront {
        inner: TubeDecal {
            origin: centre,
            along: up,
            side: front,
            across: right,
            section,
        },
    };
    // The drawing's box centred on the tube's front.
    let d = Vec2::new(-0.285, -0.125);
    let put = |p: &mut Parts, slot: &'static str, f: Flat, lift: f32| {
        decal.emit(p.slot(slot), &f.transformed(unit, d * unit), lift);
    };
    // The mane: four bands round the back of the head, the pink one
    // curling forward under the chin.
    let c = Vec2::new(0.3, 0.1);
    let arc = |radius: f32, from: f32, to: f32| -> Vec<Vec2> {
        let n = 16;
        (0..=n)
            .map(|k| {
                let t = (from + (to - from) * k as f32 / n as f32).to_radians();
                c + Vec2::new(t.cos(), t.sin()) * radius
            })
            .collect()
    };
    let step = 10.0;
    for (slot, radius) in [(PINK, 1.9f32), (YELLOW, 1.58), (BLUE, 1.26), (GREEN, 0.95)] {
        let mut path = arc(radius, 102.0, -112.0);
        if slot == PINK {
            path.extend([
                Vec2::new(-0.95, -1.55),
                Vec2::new(-1.15, -1.3),
                Vec2::new(-0.95, -1.08),
            ]);
        }
        let mut f = Flat::default();
        f.stroke(&path, 0.17, Ends::Round, step);
        put(p, slot, f, LIFT_OUTLINE);
    }
    // A blue lock over the brow and a pink one by the ears.
    let mut f = Flat::default();
    f.stroke(
        &[
            Vec2::new(-0.55, 1.0),
            Vec2::new(-1.05, 1.08),
            Vec2::new(-1.38, 0.85),
            Vec2::new(-1.3, 0.62),
        ],
        0.09,
        Ends::Round,
        step,
    );
    put(p, BLUE, f, LIFT_DETAIL);
    // The head and the muzzle, the ears.
    let mut head = Flat::default();
    head.ellipse(Vec2::new(-0.4, 0.15), Vec2::new(1.05, 0.95), 0.0, 14);
    head.ellipse(Vec2::new(-1.2, -0.35), Vec2::new(0.6, 0.48), -0.35, 12);
    head.polygon(&[
        Vec2::new(0.0, 0.9),
        Vec2::new(0.32, 1.62),
        Vec2::new(0.6, 0.8),
    ]);
    head.polygon(&[
        Vec2::new(0.4, 0.82),
        Vec2::new(0.78, 1.42),
        Vec2::new(0.88, 0.62),
    ]);
    put(p, WHITE, head, LIFT_FILL);
    // The horn, golden, with its twists.
    let mut horn = Flat::default();
    horn.polygon(&[
        Vec2::new(-0.98, 0.86),
        Vec2::new(-0.42, 1.06),
        Vec2::new(-1.38, 2.08),
    ]);
    put(p, YELLOW, horn, LIFT_DETAIL);
    // The closed eye with its lashes, the nostril and the smile.
    let mut ink = Flat::default();
    ink.stroke(
        &[
            Vec2::new(-0.66, 0.3),
            Vec2::new(-0.42, 0.17),
            Vec2::new(-0.16, 0.3),
        ],
        0.05,
        Ends::Round,
        step,
    );
    for (a, b) in [
        ((-0.58, 0.22), (-0.66, 0.06)),
        ((-0.42, 0.16), (-0.42, 0.0)),
        ((-0.26, 0.22), (-0.18, 0.07)),
        ((-1.0, 1.28), (-0.74, 1.36)),
        ((-1.12, 1.6), (-0.92, 1.67)),
        ((-1.58, -0.36), (-1.5, -0.28)),
        ((-1.38, -0.64), (-1.14, -0.68)),
    ] {
        ink.stroke(
            &[Vec2::new(a.0, a.1), Vec2::new(b.0, b.1)],
            0.035,
            Ends::Round,
            step,
        );
    }
    put(p, INK, ink, LIFT_DETAIL + 0.0002);
}

/// [`TubeDecal`] on the front of a tube (the creases cut across u).
struct TubeDecalFront {
    inner: TubeDecal,
}

impl TubeDecalFront {
    fn emit(&self, shape: &mut Shape, flat: &Flat, lift: f32) {
        let t = &self.inner;
        let mut f = flat.clone();
        for c in t.section.creases(t.side, t.across) {
            f.split(0, c);
        }
        emit_with(shape, &f, lift, |p| {
            let (q, n) = t.section.at(t.side, t.across, p.x);
            (t.origin + t.along * p.y + q, n)
        });
    }
}

/// The rim decals on both faces of a deep rim (hub `hub`, faces at
/// ±`half_width`, from radius `inner` to `outer`): "newk it" in blocks
/// round the upper front, and opposite a panel – the front rim's pink
/// with the name small in ink, the rear rim's yellow letters and a
/// small yellow block.
pub fn rim(p: &mut Parts, hub: Vec3, half_width: f32, inner: f32, outer: f32, rear: bool) {
    let depth = outer - inner;
    let mid = 0.5 * (inner + outer);
    let colour = if rear { YELLOW } else { PINK };
    let xh = depth * 0.46;
    let (letters, _) = text(NAME, xh, 0.36, 0.0, 0.0, Ends::Square, 0.008);
    for s in [-1.0f32, 1.0] {
        let x = s * half_width;
        // The letters round the upper front (45° ahead of the top).
        let d = RimDecal {
            hub,
            x,
            radius: mid,
            at: -PI / 4.0,
        };
        d.emit(p.slot(colour), &letters, LIFT_FILL * 0.6);
        // The panel opposite, low behind.
        let panel = RimDecal {
            hub,
            x,
            radius: mid,
            at: 3.0 * PI / 4.0,
        };
        let len = if rear { 0.03 } else { 0.13 };
        let mut f = Flat::default();
        f.stroke(
            &[Vec2::new(-len * 0.5, 0.0), Vec2::new(len * 0.5, 0.0)],
            depth * 0.36,
            Ends::Square,
            0.008,
        );
        panel.emit(p.slot(colour), &f, LIFT_FILL * 0.6);
        if !rear {
            let (small, _) = text(NAME, depth * 0.2, 0.3, 0.0, 0.18, Ends::Square, 0.006);
            panel.emit(p.slot(INK), &small, LIFT_FILL);
        }
    }
}

/// The fork's pink inner faces: a pink strip along the inside of each
/// blade (centre line `path`, radii `radii`, `side` −1 / +1).
pub fn fork_inner(p: &mut Parts, path: &[Vec3], radii: &[f32], side: f32, sides: u32) {
    let inner: Vec<Vec3> = path
        .iter()
        .zip(radii)
        .map(|(c, r)| *c - Vec3::X * side * r * 0.42)
        .collect();
    let r: Vec<f32> = radii.iter().map(|r| r * 0.62).collect();
    p.slot(PINK).sweep(&inner, &r, sides, true);
}

/// The green anodised thru-axle ends and the rotor's lock ring of a hub.
pub fn green_hub(p: &mut Parts, hub: Vec3, disc: bool) {
    for s in [-1.0f32, 1.0] {
        p.slot(ANODISED).tube(
            hub + Vec3::X * s * 0.05,
            hub + Vec3::X * s * 0.062,
            0.011,
            0.01,
            8,
            true,
        );
    }
    if disc {
        p.slot(ANODISED)
            .annulus(hub - Vec3::X * 0.051, Vec3::X, 0.012, 0.021, 0.003, 12);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_lays_out_left_to_right() {
        let (strokes, width) = layout(NAME, 0.3, 0.0);
        assert_eq!(strokes.len(), 12, "n 2, e 1, w 2, k 3, i 2, t 2");
        assert!(width > 7.0 && width < 10.0, "{width}");
        let first = strokes[0][0].x;
        let last = strokes.last().unwrap()[0].x;
        assert!(first < last);
    }

    #[test]
    fn strokes_and_splits_keep_the_area() {
        let mut f = Flat::default();
        f.stroke(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
            ],
            0.1,
            Ends::Square,
            0.25,
        );
        let area = |f: &Flat| -> f32 {
            f.tris
                .iter()
                .map(|t| {
                    let p = t.map(|i| f.pts[i as usize]);
                    0.5 * (p[1] - p[0]).perp_dot(p[2] - p[0])
                })
                .sum()
        };
        let a0 = area(&f);
        assert!(a0 > 0.0, "counter-clockwise");
        f.split(0, 0.5);
        f.split(1, 0.3);
        assert!((area(&f) - a0).abs() < 1e-5);
        for t in &f.tris {
            let p = t.map(|i| f.pts[i as usize]);
            for (axis, at) in [(0, 0.5f32), (1, 0.3)] {
                let s: Vec<f32> = p.iter().map(|q| q[axis] - at).collect();
                assert!(
                    s.iter().all(|v| *v >= -1e-6) || s.iter().all(|v| *v <= 1e-6),
                    "a triangle crosses the cut"
                );
            }
        }
    }

    /// Decals lie on the drawn tube: every vertex on (a hair above) a
    /// facet of the polygonal section, the normals out.
    #[test]
    fn tube_decals_lie_on_the_facets() {
        let a = Vec3::new(0.0, 0.3, 0.1);
        let b = Vec3::new(0.0, 0.75, -0.45);
        let (r, aero) = (0.0184, 2.1);
        let mut parts = Parts::new();
        down_tube_text(&mut parts, a, b, r, aero, 8);
        let s = Section::of_tube(b - a, r, aero, 8);
        let axis = (b - a).normalize();
        let shape = parts.slot(PINK);
        assert!(shape.triangles() > 50);
        for (p, n) in shape.positions.iter().zip(&shape.normals) {
            let q = *p - a;
            let q = q - axis * q.dot(axis);
            let c = Vec2::new(q.dot(s.e1), q.dot(s.e2));
            // Within the ellipse's bounding box, outside the polygon's
            // inscribed circle; the normal points away from the axis.
            assert!(c.x.abs() <= s.a + 0.002 && c.y.abs() <= s.b + 0.002);
            assert!(n.dot(q) > 0.0);
            assert!(q.length() > s.a * 0.7);
        }
    }

    #[test]
    fn the_livery_travels_as_the_frame_colour() {
        let none: Vec<String> = Vec::new();
        assert_eq!(with_paint(none.clone(), None), none);
        assert_eq!(
            with_paint(none.clone(), Some(ocw_core::bike::NEWKIT_LIVERY)),
            vec![PAINT_VARIANT.to_string()]
        );
        assert_eq!(
            with_paint(none.clone(), Some(ocw_core::Rgb(0xFF0000))),
            none
        );
        let frame = vec![FRAME_VARIANT.to_string()];
        assert_eq!(
            with_paint(frame.clone(), Some(ocw_core::Rgb(0xFF0000))),
            vec![FRAME_VARIANT.to_string(), PAINT_PLAIN.to_string()]
        );
        assert_eq!(
            default_paint(FRAME_VARIANT),
            Some(ocw_core::bike::NEWKIT_LIVERY)
        );
        assert_eq!(default_paint("bike-road-aero"), None);
    }
}
