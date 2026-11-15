//! The generated helmets of the rider (spec §2.6, v3.40; own work, MIT OR
//! Apache-2.0, built from plain points like [`crate::mesh`]).
//!
//! - The road-style helmets – the vented road helmet (the base kit), the
//!   lightweight and the aero road helmet – are a sculpted shell over the
//!   head ([`Dome`]): lanes from side to side, each running from the brow
//!   over the crown to the nape, a rear edge that juts out behind the
//!   head, vents cut into the lanes (recessed, with walls, in the dark
//!   detail colour), a dark lower band, the foam rim along the open edge,
//!   the retention band with its dial at the back and the straps.
//! - The time-trial helmet is a long teardrop lofted along the head
//!   ([`tt_grid`]): ear covers, the tail running down the back in the TT
//!   position, the integrated visor (the `lens` slot: mirrored) wrapping
//!   from temple to temple, two small intakes over the brow.
//!
//! The shell is the `helmet` slot (the rider's helmet colour, its finish
//! and the jersey designer's pattern, symmetric about x = 0 so the
//! pattern's centre stripe runs along the middle lane); vents, rim, band
//! and straps are `helmet_detail` (matte black). The far LOD gets the
//! shell alone, a few dozen triangles. Looks only: the helmets' physics
//! is the catalogue's (`assets/parts.toml`, `ocw_sim::build`).

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::{Quat, Vec2, Vec3};

use crate::mesh::Parts;
use crate::pose::nodes::{HELMET_AERO, HELMET_ROAD};
use crate::rider::{Detail, opt};

/// The paint slots of a helmet.
pub const SHELL: &str = "helmet";
pub const DETAIL: &str = "helmet_detail";
pub const LENS: &str = "lens";

/// How deep the vents are cut into the shell (m).
pub const VENT_DEPTH: f32 = 0.009;
/// The foam rim along the shell's open edge (m).
pub const RIM: f32 = 0.014;
/// Triangle budgets of one helmet: up close and in the far LOD.
pub const MAX_TRIANGLES: usize = 900;
pub const MAX_TRIANGLES_FAR: usize = 64;

/// The helmets of the rider model (the kit slot `helmet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Helmet {
    /// The vented road helmet (base kit).
    Road,
    /// The lightweight vented road helmet: more and larger vents.
    Light,
    /// The aero road helmet: a smooth, closed shell, a few ports.
    AeroRoad,
    /// The time-trial helmet: long tail and visor.
    TimeTrial,
}

impl Helmet {
    pub const ALL: [Helmet; 4] = [
        Helmet::Road,
        Helmet::Light,
        Helmet::AeroRoad,
        Helmet::TimeTrial,
    ];

    /// The model node showing it.
    pub fn node(self) -> &'static str {
        match self {
            Helmet::Road => HELMET_ROAD,
            Helmet::Light => opt::HELMET_LIGHT,
            Helmet::AeroRoad => opt::HELMET_AERO_ROAD,
            Helmet::TimeTrial => HELMET_AERO,
        }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What a cell of a helmet's grid is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// No surface (the face opening of the TT helmet).
    Skip,
    /// Surface in a paint slot.
    Paint(&'static str),
    /// A vent: [`VENT_DEPTH`] under the surface, walls to its neighbours.
    Vent,
}

/// A helmet's surface: `pts[row][col]`; cell (r, c) spans rows r, r + 1 and
/// columns c, c + 1 (`wrap`: the last column joins the first, a closed
/// ring).
#[derive(Debug, Clone)]
pub struct Grid {
    pub pts: Vec<Vec<Vec3>>,
    pub wrap: bool,
}

type Ix = (usize, usize);

impl Grid {
    fn cols(&self) -> usize {
        self.pts[0].len()
    }

    /// Cells across and around.
    pub fn cells(&self) -> (usize, usize) {
        let around = if self.wrap {
            self.cols()
        } else {
            self.cols() - 1
        };
        (self.pts.len() - 1, around)
    }

    fn at(&self, (r, c): Ix) -> Vec3 {
        self.pts[r][c]
    }

    /// The corners of cell (r, c): (r, c), (r, c+1), (r+1, c), (r+1, c+1).
    fn corners(&self, r: usize, c: usize) -> [Ix; 4] {
        let c1 = (c + 1) % self.cols();
        [(r, c), (r, c1), (r + 1, c), (r + 1, c1)]
    }

    pub fn centre(&self, r: usize, c: usize) -> Vec3 {
        self.corners(r, c).iter().map(|&i| self.at(i)).sum::<Vec3>() / 4.0
    }

    /// The neighbour of cell (r, c) across its edges (bottom, top, left,
    /// right), `None` off the grid.
    fn neighbours(&self, r: usize, c: usize) -> [(Ix, Ix, Option<Ix>); 4] {
        let (nr, nc) = self.cells();
        let [a, b, d, e] = self.corners(r, c);
        let left = if c > 0 {
            Some((r, c - 1))
        } else if self.wrap {
            Some((r, nc - 1))
        } else {
            None
        };
        let right = if c + 1 < nc {
            Some((r, c + 1))
        } else if self.wrap {
            Some((r, 0))
        } else {
            None
        };
        [
            (a, b, r.checked_sub(1).map(|r| (r, c))),
            (d, e, (r + 1 < nr).then_some((r + 1, c))),
            (a, d, left),
            (b, e, right),
        ]
    }

    /// The painted grid: cell kinds, whether the winding flips (outward
    /// faces seen from `inside`) and the vertex normals.
    fn layout(&self, inside: Vec3, cell: &impl Fn(usize, usize, Vec3) -> Cell) -> Layout {
        let (nr, nc) = self.cells();
        let kinds: Vec<Vec<Cell>> = (0..nr)
            .map(|r| (0..nc).map(|c| cell(r, c, self.centre(r, c))).collect())
            .collect();
        let tris = |r: usize, c: usize| {
            let [a, b, d, e] = self.corners(r, c);
            [[a, b, d], [b, e, d]]
        };
        let normal = |t: [Ix; 3]| {
            let [p, q, s] = t.map(|i| self.at(i));
            (q - p).cross(s - p)
        };
        // Every cell (r, c) with its kind, row by row.
        let all = || {
            kinds
                .iter()
                .enumerate()
                .flat_map(|(r, row)| row.iter().enumerate().map(move |(c, k)| (r, c, *k)))
        };
        let mut sum = 0.0;
        for (r, c, _) in all().filter(|&(_, _, k)| k != Cell::Skip) {
            for t in tris(r, c) {
                let centroid = t.iter().map(|&i| self.at(i)).sum::<Vec3>() / 3.0;
                sum += normal(t).dot(centroid - inside);
            }
        }
        let flip = sum < 0.0;
        let mut normals = vec![vec![Vec3::ZERO; self.cols()]; self.pts.len()];
        let mut oriented = Vec::new();
        for (r, c, kind) in all() {
            let mut two = tris(r, c);
            if flip {
                for t in &mut two {
                    t.swap(1, 2);
                }
            }
            if kind != Cell::Skip {
                for t in two {
                    let n = normal(t);
                    for (i, j) in t {
                        normals[i][j] += n;
                    }
                }
            }
            oriented.push(two);
        }
        for row in &mut normals {
            for n in row {
                *n = n.normalize_or(Vec3::Y);
            }
        }
        Layout {
            kinds,
            oriented,
            normals,
            cols: nc,
        }
    }

    /// The outer surface's triangles (every cell but the skipped ones, at
    /// the surface), outward wound: for tests (clipping).
    pub fn triangles(
        &self,
        inside: Vec3,
        cell: impl Fn(usize, usize, Vec3) -> Cell,
    ) -> Vec<[Vec3; 3]> {
        let l = self.layout(inside, &cell);
        let mut out = Vec::new();
        for (k, two) in l.oriented.iter().enumerate() {
            if l.kinds[k / l.cols][k % l.cols] == Cell::Skip {
                continue;
            }
            for t in two {
                let t = t.map(|i| self.at(i));
                if (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-14 {
                    out.push(t);
                }
            }
        }
        out
    }

    /// Adds the surface to `p`: painted cells smooth per slot, vents
    /// recessed with walls, a [`RIM`] (`rim` thick, 0: none) along the
    /// open edges of painted cells (not the visor's).
    pub fn emit(
        &self,
        p: &mut Parts,
        inside: Vec3,
        rim: f32,
        cell: impl Fn(usize, usize, Vec3) -> Cell,
    ) {
        let l = self.layout(inside, &cell);
        let kind = |i: Option<Ix>| i.map_or(Cell::Skip, |(r, c)| l.kinds[r][c]);
        let inner = |i: Ix, d: f32| self.at(i) - l.normals[i.0][i.1] * d;
        let area = |t: &[Vec3; 3]| (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-14;
        let mut smooth: BTreeMap<&'static str, (Vec<Vec3>, Vec<[u32; 3]>, HashMap<Ix, u32>)> =
            BTreeMap::new();
        let mut flat: Vec<[Vec3; 3]> = Vec::new();
        // A quad facing towards `towards` (two flat triangles).
        let quad = |q: [Vec3; 4], towards: Vec3, flat: &mut Vec<[Vec3; 3]>| {
            let mid = q.iter().sum::<Vec3>() / 4.0;
            let n = (q[1] - q[0]).cross(q[2] - q[0]) + (q[2] - q[0]).cross(q[3] - q[0]);
            let tris = if n.dot(towards - mid) >= 0.0 {
                [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]
            } else {
                [[q[0], q[2], q[1]], [q[0], q[3], q[2]]]
            };
            flat.extend(tris.into_iter().filter(area));
        };
        for (k, &two) in l.oriented.iter().enumerate() {
            let (r, c) = (k / l.cols, k % l.cols);
            match l.kinds[r][c] {
                Cell::Skip => {}
                Cell::Paint(slot) => {
                    let (verts, tris, map) = smooth.entry(slot).or_default();
                    for t in two {
                        if !area(&t.map(|i| self.at(i))) {
                            continue;
                        }
                        tris.push(t.map(|i| {
                            *map.entry(i).or_insert_with(|| {
                                verts.push(self.at(i));
                                verts.len() as u32 - 1
                            })
                        }));
                    }
                    if rim > 0.0 && slot != LENS {
                        let centre = self.centre(r, c);
                        for (a, b, nb) in self.neighbours(r, c) {
                            if kind(nb) != Cell::Skip || self.at(a).distance(self.at(b)) < 0.003 {
                                continue;
                            }
                            let mid = 0.5 * (self.at(a) + self.at(b));
                            quad(
                                [self.at(a), self.at(b), inner(b, rim), inner(a, rim)],
                                mid + (mid - centre),
                                &mut flat,
                            );
                        }
                    }
                }
                Cell::Vent => {
                    let floor = two.map(|t| t.map(|i| inner(i, VENT_DEPTH)));
                    flat.extend(floor.into_iter().filter(area));
                    let centre = self
                        .corners(r, c)
                        .iter()
                        .map(|&i| inner(i, VENT_DEPTH))
                        .sum::<Vec3>()
                        / 4.0;
                    for (a, b, nb) in self.neighbours(r, c) {
                        if kind(nb) == Cell::Vent || self.at(a).distance(self.at(b)) < 1e-6 {
                            continue;
                        }
                        quad(
                            [
                                self.at(a),
                                self.at(b),
                                inner(b, VENT_DEPTH),
                                inner(a, VENT_DEPTH),
                            ],
                            centre,
                            &mut flat,
                        );
                    }
                }
            }
        }
        for (slot, (verts, tris, _)) in smooth {
            p.slot(slot).add_smooth(&verts, &tris);
        }
        if !flat.is_empty() {
            let verts: Vec<Vec3> = flat.iter().flatten().copied().collect();
            let tris: Vec<[u32; 3]> = (0..flat.len() as u32)
                .map(|k| [3 * k, 3 * k + 1, 3 * k + 2])
                .collect();
            p.slot(DETAIL).add_flat(&verts, &tris);
        }
    }
}

struct Layout {
    kinds: Vec<Vec<Cell>>,
    oriented: Vec<[[Ix; 3]; 2]>,
    normals: Vec<Vec<Vec3>>,
    cols: usize,
}

/// (y, z) of the direction `b` in the head's middle plane: 0 to the front
/// (−z), π/2 up, π to the back.
fn dir(b: f32) -> Vec2 {
    Vec2::new(b.sin(), -b.cos())
}

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// A road-style helmet's shell: an egg over the head, its outline in the
/// middle plane an ellipse (radii `up`, `front`, `back`; tucked in under
/// the back of the head) drawn out to a rear edge (`tail`: the outline is
/// the hull of the ellipse and that point – the sharp rear profile), its
/// width tapering to the back (a teardrop from above). The lanes run from
/// side to side at latitudes φ about the x axis through `centre`, each from
/// the brow (`brow` at the middle, `temples` lower at the sides) to the
/// nape (`nape`, `ears` higher at the sides).
#[derive(Debug, Clone, Copy)]
pub struct Dome {
    pub centre: Vec3,
    pub width: f32,
    pub taper: f32,
    pub up: f32,
    pub front: f32,
    pub back: f32,
    /// Share of `back` lost per radian below the horizontal at the back.
    pub tuck: f32,
    /// The rear edge: direction (b) and distance from the centre.
    pub tail: Option<(f32, f32)>,
    pub brow: f32,
    pub temples: f32,
    pub nape: f32,
    pub ears: f32,
}

impl Dome {
    /// The ellipse's distance from the centre in the direction `b`.
    fn base(&self, b: f32) -> f32 {
        let (s, c) = b.sin_cos();
        let rz = if c >= 0.0 {
            self.front
        } else {
            self.back * (1.0 - self.tuck * (b - PI).max(0.0))
        };
        1.0 / ((s / self.up).powi(2) + (c / rz).powi(2)).sqrt()
    }

    /// The rear edge point and the two points where the outline leaves
    /// the ellipse for it (the tangents from the edge point).
    fn hull(&self) -> Option<(Vec2, Vec2, Vec2)> {
        let (tb, tl) = self.tail?;
        let t = dir(tb) * tl;
        let axis = -t.normalize();
        let (mut hi, mut lo) = ((f32::MIN, t), (f32::MAX, t));
        for i in 0..=720 {
            let b = -1.0 + (PI + 2.2) * i as f32 / 720.0;
            let q = dir(b) * self.base(b);
            let d = q - t;
            let a = cross2(axis, d).atan2(axis.dot(d));
            if a > hi.0 {
                hi = (a, q);
            }
            if a < lo.0 {
                lo = (a, q);
            }
        }
        Some((t, hi.1, lo.1))
    }

    /// The outline's distance from the centre in the direction `b`.
    fn radius(&self, b: f32, hull: Option<(Vec2, Vec2, Vec2)>) -> f32 {
        let mut r = self.base(b);
        if let Some((t, a1, a2)) = hull {
            let d = dir(b);
            for a in [a1, a2] {
                let e = a - t;
                let den = cross2(d, e);
                if den.abs() < 1e-9 {
                    continue;
                }
                let s = cross2(t, e) / den;
                let u = cross2(t, d) / den;
                if s > 0.0 && (0.0..=1.0).contains(&u) {
                    r = r.max(s);
                }
            }
        }
        r
    }

    fn width_at(&self, b: f32) -> f32 {
        self.width * (1.0 - self.taper * smoothstep(FRAC_PI_2, PI + 0.3, b))
    }

    /// The shell's points: `lanes` the latitudes between the lanes from
    /// the middle (0) out to the side (π/2, where the lanes meet), mirrored
    /// to the left; `cols` cells from the brow to the nape.
    pub fn grid(&self, lanes: &[f32], cols: usize) -> Grid {
        let hull = self.hull();
        let phis: Vec<f32> = lanes[1..]
            .iter()
            .rev()
            .map(|p| -p)
            .chain(lanes.iter().copied())
            .collect();
        let pts = phis
            .iter()
            .map(|&phi| {
                let side = smoothstep(0.75, FRAC_PI_2, phi.abs());
                let lo = self.brow - self.temples * side;
                let hi = self.nape - self.ears * side;
                (0..=cols)
                    .map(|m| {
                        let b = lo + (hi - lo) * m as f32 / cols as f32;
                        let d = dir(b) * self.radius(b, hull);
                        let (s, c) = phi.sin_cos();
                        self.centre + Vec3::new(self.width_at(b) * s, c * d.x, c * d.y)
                    })
                    .collect()
            })
            .collect();
        Grid { pts, wrap: false }
    }
}

/// A road-style helmet: its shell, lanes and vents.
struct RoadStyle {
    dome: Dome,
    /// Latitudes between the lanes (half, from the middle).
    lanes: &'static [f32],
    cols: usize,
    /// Vents: (lane, from, to), lane 0 the middle one, from / to the share
    /// of the way from the brow (0) to the nape (1).
    vents: &'static [(usize, f32, f32)],
}

impl RoadStyle {
    /// The cell (r, c): the outer lane and the nape row dark, the vents,
    /// the rest the shell.
    fn cell(&self, r: usize, c: usize) -> Cell {
        let half = self.lanes.len() - 1;
        let lane = if r >= half { r - half } else { half - 1 - r };
        let s = (c as f32 + 0.5) / self.cols as f32;
        if lane == half - 1 || c == self.cols - 1 {
            Cell::Paint(DETAIL)
        } else if self
            .vents
            .iter()
            .any(|&(l, from, to)| l == lane && (from..=to).contains(&s))
        {
            Cell::Vent
        } else {
            Cell::Paint(SHELL)
        }
    }
}

fn road_style(h: Helmet, skull: Vec3) -> RoadStyle {
    let centre = skull + Vec3::new(0.0, -0.005, 0.012);
    match h {
        Helmet::Light => RoadStyle {
            dome: Dome {
                centre,
                width: 0.104,
                taper: 0.1,
                up: 0.13,
                front: 0.13,
                back: 0.146,
                tuck: 0.35,
                tail: Some((PI - 0.18, 0.17)),
                brow: 0.5,
                temples: 0.78,
                nape: PI + 0.22,
                ears: 0.3,
            },
            lanes: &[0.0, 0.2, 0.38, 0.52, 0.72, 0.86, 1.06, 1.22, FRAC_PI_2],
            cols: 12,
            vents: &[
                (1, 0.06, 0.3),
                (1, 0.36, 0.56),
                (1, 0.62, 0.86),
                (3, 0.06, 0.3),
                (3, 0.36, 0.56),
                (3, 0.62, 0.86),
                (5, 0.1, 0.3),
                (5, 0.4, 0.56),
                (5, 0.62, 0.8),
            ],
        },
        Helmet::AeroRoad => RoadStyle {
            dome: Dome {
                centre,
                width: 0.102,
                taper: 0.2,
                up: 0.13,
                front: 0.13,
                back: 0.142,
                tuck: 0.3,
                tail: Some((PI + 0.02, 0.185)),
                brow: 0.46,
                temples: 0.75,
                nape: PI + 0.25,
                ears: 0.3,
            },
            lanes: &[0.0, 0.2, 0.42, 0.6, 0.85, 1.15, FRAC_PI_2],
            cols: 12,
            vents: &[(1, 0.03, 0.2), (3, 0.06, 0.2), (1, 0.8, 0.9)],
        },
        _ => RoadStyle {
            dome: Dome {
                centre,
                width: 0.104,
                taper: 0.12,
                up: 0.13,
                front: 0.13,
                back: 0.138,
                tuck: 0.35,
                tail: Some((PI - 0.12, 0.165)),
                brow: 0.48,
                temples: 0.75,
                nape: PI + 0.25,
                ears: 0.3,
            },
            lanes: &[0.0, 0.2, 0.42, 0.58, 0.82, 0.98, 1.22, FRAC_PI_2],
            cols: 12,
            vents: &[
                (1, 0.1, 0.5),
                (1, 0.58, 0.82),
                (3, 0.14, 0.48),
                (3, 0.6, 0.8),
                (5, 0.2, 0.42),
            ],
        },
    }
}

/// The far LOD's lanes and columns of a road-style shell.
const FAR_LANES: [f32; 3] = [0.0, 0.8, FRAC_PI_2];
const FAR_COLS: usize = 4;

/// The time-trial helmet's rings from the front to the tail (relative to
/// the skull's centre): z, top, bottom, half width.
const TT_RINGS: [(f32, f32, f32, f32); 10] = [
    (-0.124, 0.066, -0.044, 0.062),
    (-0.102, 0.104, -0.064, 0.086),
    (-0.068, 0.126, -0.082, 0.099),
    (-0.025, 0.135, -0.094, 0.105),
    (0.03, 0.128, -0.098, 0.104),
    (0.085, 0.11, -0.09, 0.093),
    (0.14, 0.072, -0.08, 0.073),
    (0.195, 0.02, -0.082, 0.05),
    (0.245, -0.035, -0.09, 0.03),
    (0.28, -0.068, -0.093, 0.016),
];
const TT_NOSE: Vec3 = Vec3::new(0.0, 0.012, -0.138);
const TT_TIP: Vec3 = Vec3::new(0.0, -0.088, 0.305);
/// How far the tail is turned down about the skull's centre (rad; < 0:
/// up): tuned so it runs along the rider's back in the TT position.
pub const TT_DROOP: f32 = -0.05;

/// The time-trial helmet's points (relative rings, the nose and the tail
/// tip as single-point rows): `sides` points round each ring.
pub fn tt_grid(skull: Vec3, detail: Detail) -> Grid {
    let (rings, sides): (Vec<_>, usize) = match detail {
        Detail::High => (TT_RINGS.to_vec(), 20),
        Detail::Low => (vec![TT_RINGS[2], TT_RINGS[5], TT_RINGS[8]], 8),
    };
    let turn = |q: Vec3| {
        let a = TT_DROOP * smoothstep(0.0, 0.3, q.z);
        skull + Quat::from_rotation_x(a) * q
    };
    let mut pts = vec![vec![turn(TT_NOSE); sides]];
    for (z, top, bottom, hw) in rings {
        let (cy, hh) = (0.5 * (top + bottom), 0.5 * (top - bottom));
        pts.push(
            (0..sides)
                .map(|k| {
                    let t = TAU * k as f32 / sides as f32;
                    turn(Vec3::new(hw * t.cos(), cy + hh * t.sin(), z))
                })
                .collect(),
        );
    }
    pts.push(vec![turn(TT_TIP); sides]);
    Grid { pts, wrap: true }
}

/// The time-trial helmet's cell at `at` (relative to the skull): the
/// visor round the front below the brow, the face opening and the open
/// underside down to the neck, two intakes over the brow.
fn tt_cell(rel: Vec3, hi: bool) -> Cell {
    if rel.y < -0.052 && rel.z < 0.14 {
        Cell::Skip
    } else if rel.z < -0.03 && rel.y < 0.06 {
        Cell::Paint(LENS)
    } else if hi
        && (-0.115..-0.06).contains(&rel.z)
        && rel.y > 0.085
        && (0.022..0.06).contains(&rel.x.abs())
    {
        Cell::Vent
    } else {
        Cell::Paint(SHELL)
    }
}

/// The surface of helmet `h` and its cells (the far LOD: the shell only).
pub fn surface(
    h: Helmet,
    skull: Vec3,
    detail: Detail,
) -> (Grid, Box<dyn Fn(usize, usize, Vec3) -> Cell>) {
    let hi = detail == Detail::High;
    if h == Helmet::TimeTrial {
        return (
            tt_grid(skull, detail),
            Box::new(move |_, _, at| tt_cell(at - skull, hi)),
        );
    }
    let style = road_style(h, skull);
    if !hi {
        return (
            style.dome.grid(&FAR_LANES, FAR_COLS),
            Box::new(|_, _, _| Cell::Paint(SHELL)),
        );
    }
    (
        style.dome.grid(style.lanes, style.cols),
        Box::new(move |r, c, _| style.cell(r, c)),
    )
}

/// The retention band round the back of the head below the shell, its
/// dial at the back.
fn retention(p: &mut Parts, skull: Vec3) {
    let path: Vec<Vec3> = [-100.0f32, -66.0, -33.0, 0.0, 33.0, 66.0, 100.0]
        .iter()
        .map(|d| {
            let (s, c) = d.to_radians().sin_cos();
            skull + Vec3::new(0.08 * s, -0.066 + 0.046 * (1.0 - c), 0.02 + 0.092 * c)
        })
        .collect();
    p.slot(DETAIL).sweep(&path, &[0.0045; 7], 4, false);
    p.slot(DETAIL).tube(
        skull + Vec3::new(0.0, -0.066, 0.108),
        skull + Vec3::new(0.0, -0.066, 0.122),
        0.013,
        0.012,
        6,
        true,
    );
}

/// The straps: in front of and behind each ear down to the splitter
/// under it, on to the buckle under the chin.
fn straps(p: &mut Parts, skull: Vec3) {
    for s in [-1.0f32, 1.0] {
        let v = |x: f32, y: f32, z: f32| skull + Vec3::new(s * x, y, z);
        let split = v(0.077, -0.056, -0.004);
        p.slot(DETAIL)
            .tube(v(0.085, 0.014, -0.04), split, 0.0035, 0.0035, 3, false);
        p.slot(DETAIL)
            .tube(v(0.083, -0.01, 0.05), split, 0.0035, 0.0035, 3, false);
        p.slot(DETAIL).sweep(
            &[split, v(0.062, -0.086, -0.026), v(0.02, -0.112, -0.032)],
            &[0.0035; 3],
            3,
            false,
        );
    }
    p.slot(DETAIL).cuboid(
        skull + Vec3::new(0.0, -0.113, -0.032),
        Vec3::new(0.021, 0.004, 0.007),
        Quat::IDENTITY,
    );
}

/// Adds helmet `h` on a head whose skull is centred at `skull` (rest-pose
/// model coordinates).
pub fn build(p: &mut Parts, h: Helmet, skull: Vec3, detail: Detail) {
    let hi = detail == Detail::High;
    let (grid, cell) = surface(h, skull, detail);
    grid.emit(p, skull, if hi { RIM } else { 0.0 }, cell);
    if hi {
        if h != Helmet::TimeTrial {
            retention(p, skull);
        }
        straps(p, skull);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;
    use crate::pose::nodes::{CAP, HEAD, TORSO};
    use crate::rider;

    /// The triangles of node `name` in rest-pose model coordinates
    /// (`slot`: only that material's).
    fn node_tris(m: &Model, name: &str, slot: Option<&str>) -> Vec<[Vec3; 3]> {
        let i = m.node(name).unwrap_or_else(|| panic!("{name}"));
        let w = m.world(i);
        m.nodes[i]
            .primitives
            .iter()
            .filter(|p| slot.is_none_or(|s| m.materials[p.material].name == s))
            .flat_map(|p| {
                p.indices.chunks(3).map(move |t| {
                    [0, 1, 2].map(|k| w.transform_point3(Vec3::from(p.positions[t[k] as usize])))
                })
            })
            .collect()
    }

    /// Where the segment p0 → p1 crosses triangle `t` (share of the way).
    fn crossing(p0: Vec3, p1: Vec3, t: &[Vec3; 3]) -> Option<f32> {
        let d = p1 - p0;
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let h = d.cross(e2);
        let a = e1.dot(h);
        if a.abs() < 1e-12 {
            return None;
        }
        let f = 1.0 / a;
        let s = p0 - t[0];
        let u = f * s.dot(h);
        let q = s.cross(e1);
        let v = f * d.dot(q);
        if !(0.0..=1.0).contains(&u) || v < 0.0 || u + v > 1.0 {
            return None;
        }
        Some(f * e2.dot(q))
    }

    fn shell(h: Helmet) -> Vec<[Vec3; 3]> {
        let skull = rider::skull_centre();
        let (g, cell) = surface(h, skull, Detail::High);
        g.triangles(skull, cell)
    }

    /// Up close a helmet stays in its budget, far away it is a few dozen
    /// triangles (the base helmet no more than before v3.40); the whole
    /// rider model stays in the validator's budget.
    #[test]
    fn helmets_stay_in_their_triangle_budgets() {
        let hi = rider::build(Detail::High);
        let lo = rider::build(Detail::Low);
        for h in Helmet::ALL {
            let near = hi.triangles_below(hi.node(h.node()));
            let far = lo.triangles_below(lo.node(h.node()));
            eprintln!("{h:?}: {near} triangles, far {far}");
            assert!((200..=MAX_TRIANGLES).contains(&near), "{h:?}: {near}");
            assert!((8..=MAX_TRIANGLES_FAR).contains(&far), "{h:?}: {far}");
        }
        let base_far = lo.triangles_below(lo.node(HELMET_ROAD));
        assert!(base_far <= 48, "far base helmet: {base_far}");
        eprintln!("rider: {} triangles", hi.triangles());
        assert!(hi.triangles() <= crate::manifest::MAX_TRIANGLES_RIDER);
    }

    /// The shell is the helmet slot (colour, finish, the designer's
    /// pattern), symmetric about the middle with its middle lane on top
    /// (the pattern's stripe, |x| < 2 cm); vents, rim and straps are the
    /// detail slot; the TT helmet's visor is a lens.
    #[test]
    fn shells_are_symmetric_and_carry_the_pattern() {
        let m = rider::build(Detail::High);
        let skull = rider::skull_centre();
        for h in Helmet::ALL {
            let shell = node_tris(&m, h.node(), Some(SHELL));
            assert!(!shell.is_empty(), "{h:?}");
            let pts: Vec<Vec3> = shell.iter().flatten().copied().collect();
            for p in &pts {
                let mirror = Vec3::new(-p.x, p.y, p.z);
                assert!(
                    pts.iter().any(|q| q.distance(mirror) < 1e-4),
                    "{h:?}: {p} has no mirror image"
                );
            }
            assert!(
                pts.iter().any(|p| p.x.abs() < 0.02 && p.y > skull.y + 0.1),
                "{h:?}: the stripe's lane"
            );
            assert!(!node_tris(&m, h.node(), Some(DETAIL)).is_empty(), "{h:?}");
            let lens = node_tris(&m, h.node(), Some(LENS));
            assert_eq!(!lens.is_empty(), h == Helmet::TimeTrial, "{h:?}");
        }
    }

    /// No clipping: the hair styles, faces, glasses (under the road
    /// helmets' brow), the cotton cap and the head itself never cross a
    /// helmet's shell.
    #[test]
    fn helmets_clear_hair_glasses_caps_and_the_head() {
        let m = rider::build(Detail::High);
        let items = [
            opt::HAIR_CAP,
            opt::HAIR_BUZZ,
            opt::HAIR_LONG,
            opt::HAIR_TAIL,
            opt::HAIR_BUN,
            opt::FACE_BEARD,
            opt::FACE_MOUSTACHE,
            opt::FACE_STUBBLE,
            opt::GLASSES_SPORT,
            opt::GLASSES_ROUND,
            CAP,
            HEAD,
        ];
        let mut clips = Vec::new();
        for h in Helmet::ALL {
            let shell = shell(h);
            for item in items {
                // The visor replaces the glasses and the cap
                // (rider::visible).
                if h == Helmet::TimeTrial && (item.starts_with("glasses") || item == CAP) {
                    continue;
                }
                let skull = rider::skull_centre();
                let hits: Vec<Vec3> = node_tris(&m, item, None)
                    .iter()
                    .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
                    .filter_map(|(a, b)| {
                        shell.iter().find_map(|s| {
                            crossing(a, b, s)
                                .filter(|k| (1e-4..1.0 - 1e-4).contains(k))
                                .map(|k| a + (b - a) * k - skull)
                        })
                    })
                    .collect();
                if let Some(at) = hits.first() {
                    clips.push(format!("{h:?} x {item}: {} edges, at {at}", hits.len()));
                }
            }
        }
        assert!(clips.is_empty(), "{clips:#?}");
    }

    /// The TT helmet's visor sits in front of the eyes, where glasses
    /// would be: seen from the skull's centre both glasses' lenses are
    /// behind it.
    #[test]
    fn the_visor_covers_the_glasses() {
        let m = rider::build(Detail::High);
        let skull = rider::skull_centre();
        let (g, cell) = surface(Helmet::TimeTrial, skull, Detail::High);
        let visor = g.triangles(skull, |r, c, at| match cell(r, c, at) {
            Cell::Paint(LENS) => Cell::Paint(LENS),
            _ => Cell::Skip,
        });
        assert!(!visor.is_empty());
        for glasses in [opt::GLASSES_SPORT, opt::GLASSES_ROUND] {
            for t in node_tris(&m, glasses, Some(LENS)) {
                // The lenses' front (the rest lies in the face).
                for p in t.into_iter().filter(|p| p.z < skull.z - 0.075) {
                    let far = skull + (p - skull) * 4.0;
                    let hit = visor
                        .iter()
                        .filter_map(|s| crossing(skull, far, s))
                        .any(|k| k > 0.25);
                    assert!(hit, "{glasses}: {} not behind the visor", p - skull);
                }
            }
        }
    }

    /// One helmet per look, whatever the kit: the base helmet or the worn
    /// variant.
    #[test]
    fn a_look_wears_exactly_one_helmet() {
        let c = ocw_core::Character::default();
        let optional = rider::optional();
        for h in Helmet::ALL {
            assert!(optional.contains(&h.node()), "{h:?}");
        }
        for kit in [
            vec![],
            vec!["helmet-light"],
            vec!["helmet-aero-road", "cap-cotton"],
            vec!["helmet-tt", "shoes-race"],
        ] {
            let shown = rider::visible(&c, &kit);
            let worn: Vec<Helmet> = Helmet::ALL
                .into_iter()
                .filter(|h| shown.contains(&h.node()))
                .collect();
            assert_eq!(worn.len(), 1, "{kit:?}: {worn:?}");
        }
    }

    /// The point of triangle `t` closest to `p` (Ericson, Real-Time
    /// Collision Detection 5.1.5).
    fn closest_on_triangle(p: Vec3, t: &[Vec3; 3]) -> Vec3 {
        let [a, b, c] = *t;
        let (ab, ac, ap) = (b - a, c - a, p - a);
        let (d1, d2) = (ab.dot(ap), ac.dot(ap));
        if d1 <= 0.0 && d2 <= 0.0 {
            return a;
        }
        let bp = p - b;
        let (d3, d4) = (ab.dot(bp), ac.dot(bp));
        if d3 >= 0.0 && d4 <= d3 {
            return b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            return a + ab * (d1 / (d1 - d3));
        }
        let cp = p - c;
        let (d5, d6) = (ab.dot(cp), ac.dot(cp));
        if d6 >= 0.0 && d5 <= d6 {
            return c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            return a + ac * (d2 / (d2 - d6));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
        }
        let denom = 1.0 / (va + vb + vc);
        a + ab * (vb * denom) + ac * (vc * denom)
    }

    /// On the bike in the riding pose every helmet stays clear of the
    /// back, and the TT helmet's tail runs along it in the TT position:
    /// its tip a few centimetres over the jersey.
    #[test]
    fn helmets_clear_the_back_and_the_tt_tail_follows_it() {
        use crate::pose::{self, Fit, PoseParams, RiderRig};
        use ocw_core::ProfileId;
        let rest = rider::build(Detail::High);
        let rig = RiderRig::from_model(&rest).unwrap();
        for (h, profile) in [
            (Helmet::Road, ProfileId::Road),
            (Helmet::Light, ProfileId::RoadClimbing),
            (Helmet::AeroRoad, ProfileId::RoadAero),
            (Helmet::Road, ProfileId::TimeTrial),
            (Helmet::TimeTrial, ProfileId::TimeTrial),
        ] {
            let bike = crate::bikes::rig(profile);
            let mut m = rest.clone();
            pose::pose(&rig, &bike, &Fit::new(&rig, &bike), &PoseParams::default()).apply(&mut m);
            let (torso, helmet) = (m.node(TORSO).unwrap(), m.node(h.node()).unwrap());
            let back: Vec<[Vec3; 3]> = m.nodes[torso]
                .primitives
                .iter()
                .flat_map(|p| {
                    p.indices
                        .chunks(3)
                        .map(|t| [0, 1, 2].map(|k| Vec3::from(p.positions[t[k] as usize])))
                })
                .collect();
            let to_torso = m.world(torso).inverse() * m.world(helmet);
            // Signed distance to the torso's surface (< 0: inside).
            let gap = |q: Vec3| {
                back.iter()
                    .map(|t| {
                        let c = closest_on_triangle(q, t);
                        let n = (t[1] - t[0]).cross(t[2] - t[0]);
                        let d = q.distance(c);
                        if (q - c).dot(n) < 0.0 { -d } else { d }
                    })
                    .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                    .unwrap()
            };
            let rest_w = rest.world(rest.node(h.node()).unwrap());
            let mut tip = (f32::MIN, Vec3::ZERO);
            let mut closest = (f32::MAX, Vec3::ZERO);
            for prim in &m.nodes[helmet].primitives {
                for v in &prim.positions {
                    let v = Vec3::from(*v);
                    let g = gap(to_torso.transform_point3(v));
                    if g < closest.0 {
                        closest = (g, rest_w.transform_point3(v) - rider::skull_centre());
                    }
                    let z = rest_w.transform_point3(v).z;
                    if z > tip.0 {
                        tip = (z, v);
                    }
                }
            }
            let tip_gap = gap(to_torso.transform_point3(tip.1));
            eprintln!(
                "{h:?} on the {profile:?} bike: rear edge {tip_gap:.3} m over the back, \
                 closest {:.3} m at {}",
                closest.0, closest.1
            );
            assert!(
                closest.0 > 0.0,
                "{h:?} on the {profile:?} bike: in the torso"
            );
            if h == Helmet::TimeTrial {
                assert!((0.015..0.09).contains(&tip_gap), "TT tail {tip_gap}");
            }
        }
    }
}
