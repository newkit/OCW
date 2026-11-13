//! The unicorn of god mode (spec §14 Phase 13C): our own low-poly model,
//! made here in code (licensed like the code; no third-party asset, no
//! brand character). A white body, a spiral golden horn, a rainbow mane
//! and tail, large feathered wings like a pegasus, golden hooves; the
//! rider's colour in the mane's first band (no saddle cloth).
//!
//! - The **model** ([`model`]) is a list of flat-shaded parts on a small
//!   skeleton ([`skeleton`]): body, neck, head, four legs (hip and knee),
//!   two wings (shoulder, wrist and three groups of primaries), eight mane
//!   bands, eight tail strands – about 3k triangles.
//! - The **animation** ([`AnimState::advance`], [`pose`]) is pure: a gallop
//!   in the air whose rate follows the speed (legs tucked in when fast, a
//!   gentle paddle and bob while hovering); a bird's wingbeat (spec §14
//!   Phase 13C, 2026-11-14): beating all the time in flight, 1.2 … 2 Hz
//!   ([`ocw_sim::god::wing_beat_hz`], deeper when slow or climbing,
//!   [`ocw_sim::god::wing_beat_depth`]), short glides between bursts while
//!   cruising, a quick downstroke and a slower upstroke ([`stroke`]), the
//!   hand trailing the arm, folding and sweeping back on the upstroke with
//!   the primaries closing ([`wing_angles`]), the body rising on the
//!   downstroke, neck, head and tail answering, the inner wing tucked in a
//!   bank, braking flaps when landing, then folded; each rider in its own
//!   phase; the body banking in turns, mane and tail streaming in the wind.
//! - The **transformation** (0.6 s): the unicorn grows in with a sparkle
//!   burst; a burst again where it turns back into the rider.
//! - The **landing ring**: a glowing ring on the road where leaving (G)
//!   lands ([`crate::FlightView::landing`]).
//! - The **rainbow trail** (2 s, [`UnicornTrail`]: Settings → Display →
//!   Effects) and a **name tag** over the other riders' unicorns.

use std::collections::{BTreeMap, VecDeque};
use std::f32::consts::{PI, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ocw_core::RiderId;

use crate::camera::FollowCamera;
use crate::{RideView, UnicornView, to_vec3};

pub struct UnicornPlugin;

impl Plugin for UnicornPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UnicornTrail>()
            .init_resource::<Unicorns>()
            .add_systems(
                PostUpdate,
                (
                    sync_unicorns,
                    animate,
                    impostors,
                    sparkles,
                    trails,
                    landing_ring,
                    name_tags,
                )
                    .chain()
                    .before(TransformSystems::Propagate),
            );
    }
}

/// The rainbow trail behind the unicorns (Settings → Display → Effects).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnicornTrail(pub bool);

impl Default for UnicornTrail {
    fn default() -> Self {
        Self(true)
    }
}

/// The eight colours of the mane, the tail and the trail (sRGB).
pub const RAINBOW: [u32; 8] = [
    0xE53935, 0xFB8C00, 0xFDD835, 0x43A047, 0x00ACC1, 0x1E88E5, 0x5E35B1, 0xD81B60,
];

fn srgb(c: u32) -> Color {
    Color::srgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// A joint of the unicorn's skeleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Joint {
    Body,
    Neck,
    Head,
    /// Legs: 0 front left, 1 front right, 2 hind left, 3 hind right.
    Hip(u8),
    Knee(u8),
    /// 0 left, 1 right: the shoulder (the arm) …
    Wing(u8),
    /// … the wrist (the hand, the outer wing) …
    WingTip(u8),
    /// … and the primaries on the hand, in [`PRIMARIES`] groups (side,
    /// group from the wrist out), fanning open and closed.
    Primary(u8, u8),
    /// From the head down the neck.
    Mane(u8),
    Tail(u8),
}

/// A joint, its parent and where it sits in the parent's frame (m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointDef {
    pub joint: Joint,
    pub parent: Option<Joint>,
    pub pivot: Vec3,
}

/// Mane bands and tail strands.
pub const STRANDS: u8 = 8;
/// Groups of primaries on each hand.
pub const PRIMARIES: u8 = 3;
/// Where the wrist sits along the arm (m from the shoulder) …
pub const WRIST_M: f32 = 0.78;
/// … and the primaries' root behind it (m).
pub const PRIMARY_ROOT_Z: f32 = 0.06;

/// The skeleton, parents before children. The model looks along −z, +y
/// up, +x to its right (Bevy's forward).
pub fn skeleton() -> Vec<JointDef> {
    let j = |joint, parent, pivot| JointDef {
        joint,
        parent,
        pivot,
    };
    let mut out = vec![
        j(Joint::Body, None, Vec3::ZERO),
        j(Joint::Neck, Some(Joint::Body), Vec3::new(0.0, 0.2, -0.85)),
        j(Joint::Head, Some(Joint::Neck), Vec3::new(0.0, 0.85, 0.0)),
    ];
    for leg in 0..4u8 {
        let x = if leg % 2 == 0 { -0.24 } else { 0.24 };
        let z = if leg < 2 { -0.66 } else { 0.62 };
        out.push(j(Joint::Hip(leg), Some(Joint::Body), Vec3::new(x, -0.2, z)));
        out.push(j(
            Joint::Knee(leg),
            Some(Joint::Hip(leg)),
            Vec3::new(0.0, -0.55, 0.0),
        ));
    }
    for side in 0..2u8 {
        let x = if side == 0 { -0.3 } else { 0.3 };
        out.push(j(
            Joint::Wing(side),
            Some(Joint::Body),
            Vec3::new(x, 0.36, -0.42),
        ));
        let sx = if side == 0 { -1.0 } else { 1.0 };
        out.push(j(
            Joint::WingTip(side),
            Some(Joint::Wing(side)),
            Vec3::new(sx * WRIST_M, 0.0, 0.0),
        ));
        for k in 0..PRIMARIES {
            out.push(j(
                Joint::Primary(side, k),
                Some(Joint::WingTip(side)),
                Vec3::new(0.0, 0.0, PRIMARY_ROOT_Z),
            ));
        }
    }
    for i in 0..STRANDS {
        let y = 0.78 - f32::from(i) * 0.095;
        out.push(j(
            Joint::Mane(i),
            Some(Joint::Neck),
            Vec3::new(0.0, y, 0.16),
        ));
        let x = (f32::from(i) - 3.5) * 0.025;
        out.push(j(
            Joint::Tail(i),
            Some(Joint::Body),
            Vec3::new(x, 0.22, 0.98),
        ));
    }
    out
}

/// How a part is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Paint {
    /// The white coat.
    Coat,
    /// Horn and hooves.
    Gold,
    Eye,
    Muzzle,
    /// The rider's colour: the mane's first band.
    Rider,
    /// A rainbow colour ([`RAINBOW`]).
    Band(u8),
    /// The wing feathers (pearly white).
    Feather,
}

/// A triangle mesh (indexed), in the frame of its joint.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shape {
    pub positions: Vec<Vec3>,
    pub indices: Vec<u32>,
}

impl Shape {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// The shape moved by `t`.
    pub fn transformed(mut self, t: Transform) -> Self {
        for p in &mut self.positions {
            *p = t.transform_point(*p);
        }
        self
    }

    /// Flat-shaded vertices: positions and face normals, three per
    /// triangle (the low-poly look).
    pub fn flat(&self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let mut pos = Vec::with_capacity(self.indices.len());
        let mut nor = Vec::with_capacity(self.indices.len());
        for t in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = [0, 1, 2].map(|k| self.positions[t[k] as usize]);
            let n = (b - a).cross(c - a).normalize_or_zero();
            for p in [a, b, c] {
                pos.push(p.to_array());
                nor.push(n.to_array());
            }
        }
        (pos, nor)
    }
}

/// An ellipsoid with radii `r` around the origin: `seg` sides, `rings`
/// rings (pole to pole), outward-facing triangles.
pub fn ellipsoid(r: Vec3, seg: u32, rings: u32) -> Shape {
    let (seg, rings) = (seg.max(3), rings.max(2));
    let mut positions = vec![Vec3::new(0.0, r.y, 0.0)];
    for k in 1..rings {
        let v = PI * k as f32 / rings as f32;
        for s in 0..seg {
            let u = TAU * s as f32 / seg as f32;
            positions.push(Vec3::new(
                r.x * v.sin() * u.cos(),
                r.y * v.cos(),
                r.z * v.sin() * u.sin(),
            ));
        }
    }
    positions.push(Vec3::new(0.0, -r.y, 0.0));
    let bottom = positions.len() as u32 - 1;
    let ring = |k: u32, s: u32| 1 + (k - 1) * seg + s % seg;
    let mut indices = Vec::new();
    for s in 0..seg {
        indices.extend([0, ring(1, s + 1), ring(1, s)]);
        indices.extend([bottom, ring(rings - 1, s), ring(rings - 1, s + 1)]);
    }
    for k in 1..rings - 1 {
        for s in 0..seg {
            let (a, b, c, d) = (
                ring(k, s),
                ring(k, s + 1),
                ring(k + 1, s),
                ring(k + 1, s + 1),
            );
            indices.extend([a, b, d, a, d, c]);
        }
    }
    Shape { positions, indices }
}

/// A capped frustum along +y from 0 (radius `r0`) to `len` (radius `r1`).
pub fn tube(r0: f32, r1: f32, len: f32, sides: u32) -> Shape {
    let sides = sides.max(3);
    let mut positions = Vec::new();
    for (y, r) in [(0.0, r0), (len, r1)] {
        for s in 0..sides {
            let u = TAU * s as f32 / sides as f32;
            positions.push(Vec3::new(r * u.cos(), y, r * u.sin()));
        }
    }
    positions.push(Vec3::ZERO);
    positions.push(Vec3::new(0.0, len, 0.0));
    let (bottom, top) = (2 * sides, 2 * sides + 1);
    let mut indices = Vec::new();
    for s in 0..sides {
        let (a, b) = (s, (s + 1) % sides);
        let (c, d) = (a + sides, b + sides);
        indices.extend([a, d, b, a, c, d]);
        indices.extend([bottom, a, b]);
        indices.extend([top, d, c]);
    }
    Shape { positions, indices }
}

/// The spiral horn: a cone along +y with a ridge winding `turns` times
/// round it.
pub fn horn(len: f32, r0: f32, turns: f32) -> Shape {
    const SIDES: u32 = 10;
    const STEPS: u32 = 14;
    let mut positions = Vec::new();
    for k in 0..STEPS {
        let t = k as f32 / STEPS as f32;
        for s in 0..SIDES {
            let u = TAU * s as f32 / SIDES as f32;
            let ridge = 0.82 + 0.18 * (2.0 * u - TAU * turns * t).cos();
            let r = r0 * (1.0 - t) * ridge;
            positions.push(Vec3::new(r * u.cos(), len * t, r * u.sin()));
        }
    }
    positions.push(Vec3::ZERO);
    positions.push(Vec3::new(0.0, len, 0.0));
    let (bottom, tip) = (STEPS * SIDES, STEPS * SIDES + 1);
    let at = |k: u32, s: u32| k * SIDES + s % SIDES;
    let mut indices = Vec::new();
    for s in 0..SIDES {
        indices.extend([bottom, at(0, s), at(0, s + 1)]);
        indices.extend([tip, at(STEPS - 1, s + 1), at(STEPS - 1, s)]);
        for k in 0..STEPS - 1 {
            let (a, b, c, d) = (at(k, s), at(k, s + 1), at(k + 1, s), at(k + 1, s + 1));
            indices.extend([a, d, b, a, c, d]);
        }
    }
    Shape { positions, indices }
}

/// A part of the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub joint: Joint,
    pub paint: Paint,
    pub shape: Shape,
}

fn at(t: Vec3) -> Transform {
    Transform::from_translation(t)
}

/// The unicorn's level of detail (spec §14 Phase 13C: about 3k triangles
/// near, 600 far, an impostor beyond 500 m – like the riders, §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lod {
    Near,
    Far,
    /// A camera-facing card with a drawn unicorn ([`impostor_image`]).
    Impostor,
}

/// The near model ends (the far one begins) at this distance (m) …
pub const NEAR_END_M: f32 = 80.0;
/// … the far model ends (the impostor begins) here.
pub const FAR_END_M: f32 = 500.0;
/// The models cross-fade over this part of each boundary (m, either side).
const LOD_MARGIN_M: f32 = 10.0;

impl Lod {
    /// The level drawn at `distance_m` from the camera (the middle of the
    /// cross-fade).
    pub fn at(distance_m: f32) -> Self {
        if distance_m < NEAR_END_M {
            Lod::Near
        } else if distance_m < FAR_END_M {
            Lod::Far
        } else {
            Lod::Impostor
        }
    }

    /// Bevy's visibility range of this level.
    pub fn range(self) -> bevy::camera::visibility::VisibilityRange {
        use bevy::camera::visibility::VisibilityRange;
        let m = LOD_MARGIN_M;
        match self {
            Lod::Near => VisibilityRange {
                start_margin: 0.0..0.0,
                end_margin: NEAR_END_M - m..NEAR_END_M + m,
                use_aabb: false,
            },
            Lod::Far => VisibilityRange {
                start_margin: NEAR_END_M - m..NEAR_END_M + m,
                end_margin: FAR_END_M - m..FAR_END_M + m,
                use_aabb: false,
            },
            Lod::Impostor => VisibilityRange::abrupt(FAR_END_M, f32::MAX),
        }
    }
}

/// The unicorn's parts (the near model).
pub fn model() -> Vec<Part> {
    model_lod(Lod::Near)
}

/// The unicorn's parts at a level of detail: the far model has the same
/// parts on the same joints with fewer sides, four feathers per wing and
/// no eyes; the impostor has none (it is a card).
pub fn model_lod(lod: Lod) -> Vec<Part> {
    let mut parts = Vec::new();
    let near = match lod {
        Lod::Near => true,
        Lod::Far => false,
        Lod::Impostor => return parts,
    };
    // Sides of a shape: the near count, or the far one.
    let n = |near_count: u32, far_count: u32| if near { near_count } else { far_count };
    let mut add = |joint, paint, shape: Shape| {
        parts.push(Part {
            joint,
            paint,
            shape,
        })
    };
    use Joint::*;
    // Body: barrel, chest and rump.
    add(
        Body,
        Paint::Coat,
        ellipsoid(Vec3::new(0.42, 0.46, 0.9), n(12, 6), n(8, 4)),
    );
    add(
        Body,
        Paint::Coat,
        ellipsoid(Vec3::new(0.4, 0.45, 0.45), n(10, 5), n(7, 4))
            .transformed(at(Vec3::new(0.0, 0.05, -0.62))),
    );
    add(
        Body,
        Paint::Coat,
        ellipsoid(Vec3::new(0.42, 0.44, 0.46), n(10, 5), n(7, 4))
            .transformed(at(Vec3::new(0.0, 0.04, 0.6))),
    );
    // No saddle cloth on the back (removed, decided by the user
    // 2026-11-13): the rider's colour is only the mane's first band.
    // Neck: tilted forward (rest pose in [`pose`]).
    add(Neck, Paint::Coat, tube(0.23, 0.16, 0.9, n(9, 5)));
    // Head: skull, muzzle, eyes, ears, the horn.
    add(
        Head,
        Paint::Coat,
        ellipsoid(Vec3::new(0.17, 0.2, 0.27), n(10, 5), n(7, 4))
            .transformed(at(Vec3::new(0.0, 0.05, -0.12))),
    );
    add(
        Head,
        Paint::Muzzle,
        ellipsoid(Vec3::new(0.13, 0.14, 0.2), n(9, 4), n(6, 3))
            .transformed(at(Vec3::new(0.0, -0.03, -0.42))),
    );
    for x in [-0.135, 0.135] {
        if near {
            add(
                Head,
                Paint::Eye,
                ellipsoid(Vec3::splat(0.035), 6, 4).transformed(at(Vec3::new(x, 0.11, -0.24))),
            );
        }
        add(
            Head,
            Paint::Coat,
            tube(0.05, 0.0, 0.17, n(5, 3)).transformed(Transform {
                translation: Vec3::new(x * 0.6, 0.21, -0.02),
                rotation: Quat::from_rotation_z(-x * 2.0),
                ..default()
            }),
        );
    }
    let horn_shape = if near {
        horn(0.48, 0.055, 3.0)
    } else {
        tube(0.055, 0.0, 0.48, 4)
    };
    add(
        Head,
        Paint::Gold,
        horn_shape.transformed(Transform {
            translation: Vec3::new(0.0, 0.2, -0.24),
            rotation: Quat::from_rotation_x(-0.85),
            ..default()
        }),
    );
    // Legs: upper and lower leg, golden hooves.
    for leg in 0..4u8 {
        add(
            Hip(leg),
            Paint::Coat,
            tube(0.11, 0.075, 0.55, n(7, 4)).transformed(flip()),
        );
        add(
            Knee(leg),
            Paint::Coat,
            tube(0.07, 0.06, 0.48, n(7, 4)).transformed(flip()),
        );
        add(
            Knee(leg),
            Paint::Gold,
            tube(0.085, 0.075, 0.12, n(7, 4)).transformed(Transform {
                translation: Vec3::new(0.0, -0.48, 0.0),
                rotation: Quat::from_rotation_x(PI),
                ..default()
            }),
        );
    }
    // Wings: a covert along the arm and one along the hand (the wrist
    // between them bends), and twelve flight feathers fanning from them
    // (the right wing along +x, the left mirrored) – the inner ones on the
    // arm, the primaries on the hand in [`PRIMARIES`] groups; five broader
    // ones far away.
    let feathers: u32 = n(12, 5);
    for side in 0..2u8 {
        let sx = if side == 0 { -1.0 } else { 1.0 };
        add(
            Wing(side),
            Paint::Feather,
            ellipsoid(Vec3::new(0.42, 0.06, 0.17), n(8, 4), n(6, 3)).transformed(at(Vec3::new(
                sx * 0.4,
                0.0,
                0.0,
            ))),
        );
        add(
            WingTip(side),
            Paint::Feather,
            ellipsoid(Vec3::new(0.42, 0.05, 0.15), n(8, 4), n(6, 3)).transformed(at(Vec3::new(
                sx * 0.38,
                0.0,
                0.0,
            ))),
        );
        let base_x = |t: f32| 0.12 + 1.42 * t;
        let outer: Vec<u32> = (0..feathers)
            .filter(|&k| base_x(k as f32 / (feathers - 1) as f32) >= WRIST_M)
            .collect();
        for k in 0..feathers {
            let t = k as f32 / (feathers - 1) as f32;
            let a = t.powf(1.5) * 1.15 * sx;
            let len = 0.5 + 0.55 * t.powf(0.8);
            let width = if near { 0.075 } else { 0.2 };
            let dir = Vec3::new(a.sin(), 0.0, a.cos());
            let mut base = Vec3::new(sx * base_x(t), -0.01 * k as f32 * 0.2, 0.06);
            // A primary: on its group's joint at the wrist.
            let joint = match outer.iter().position(|&o| o == k) {
                Some(i) => {
                    base -= Vec3::new(sx * WRIST_M, 0.0, PRIMARY_ROOT_Z);
                    Primary(side, (i * PRIMARIES as usize / outer.len()) as u8)
                }
                None => Wing(side),
            };
            add(
                joint,
                Paint::Feather,
                ellipsoid(Vec3::new(width, 0.016, len / 2.0), n(6, 4), n(4, 2)).transformed(
                    Transform {
                        translation: base + dir * (len / 2.0),
                        rotation: Quat::from_rotation_y(a),
                        ..default()
                    },
                ),
            );
        }
    }
    // Mane: eight bands down the back of the neck; the first in the
    // rider's colour. Tail: eight strands.
    for i in 0..STRANDS {
        let paint = if i == 0 { Paint::Rider } else { Paint::Band(i) };
        add(
            Mane(i),
            paint,
            ellipsoid(Vec3::new(0.03, 0.11, 0.15), n(6, 3), n(4, 2))
                .transformed(at(Vec3::new(0.0, 0.0, 0.11))),
        );
        add(Tail(i), Paint::Band(i), tube(0.055, 0.012, 1.1, n(5, 3)));
    }
    parts
}

/// Size of the impostor's picture (px).
pub const IMPOSTOR_PX: (u32, u32) = (64, 48);

/// The impostor's picture (RGBA8, row-major from the top): the unicorn
/// seen from the side – white body and head, raised wings, a golden horn,
/// a rainbow tail – on a transparent ground.
pub fn impostor_image() -> Vec<[u8; 4]> {
    let (w, h) = IMPOSTOR_PX;
    let mut px = vec![[0u8; 4]; (w * h) as usize];
    // Shapes in unit coordinates (x right, y up; 0 … 1).
    let ellipse = |x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32| {
        ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0
    };
    let segment = |x: f32, y: f32, a: (f32, f32), b: (f32, f32), r: f32| {
        let (abx, aby) = (b.0 - a.0, b.1 - a.1);
        let t = (((x - a.0) * abx + (y - a.1) * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
        (x - a.0 - abx * t).hypot(y - a.1 - aby * t) <= r
    };
    for j in 0..h {
        for i in 0..w {
            let x = (i as f32 + 0.5) / w as f32;
            let y = 1.0 - (j as f32 + 0.5) / h as f32;
            let mut c = None;
            // The rainbow tail streaming back (left).
            for (k, v) in RAINBOW.iter().enumerate() {
                let off = k as f32 * 0.018;
                if segment(x, y, (0.24, 0.5 - off), (0.04, 0.36 - off), 0.012) {
                    c = Some([(v >> 16) as u8, (v >> 8) as u8, *v as u8, 255]);
                }
            }
            // The wings, raised.
            if segment(x, y, (0.42, 0.55), (0.3, 0.95), 0.06)
                || segment(x, y, (0.5, 0.55), (0.46, 0.92), 0.05)
            {
                c = Some([235, 232, 245, 255]);
            }
            // Body, neck, head; legs.
            if ellipse(x, y, 0.45, 0.45, 0.22, 0.12)
                || segment(x, y, (0.62, 0.5), (0.74, 0.66), 0.05)
                || ellipse(x, y, 0.8, 0.66, 0.08, 0.05)
                || segment(x, y, (0.33, 0.38), (0.28, 0.16), 0.022)
                || segment(x, y, (0.56, 0.38), (0.64, 0.18), 0.022)
            {
                c = Some([250, 248, 252, 255]);
            }
            // The horn.
            if segment(x, y, (0.84, 0.7), (0.95, 0.86), 0.012) {
                c = Some([255, 200, 64, 255]);
            }
            if let Some(c) = c {
                px[(j * w + i) as usize] = c;
            }
        }
    }
    px
}

/// Turns a +y shape to point down (legs hang from their joints).
fn flip() -> Transform {
    Transform::from_rotation(Quat::from_rotation_x(PI))
}

/// The share of a wing beat that is the downstroke: faster (and
/// stronger) than the upstroke, as a bird's.
pub const DOWNSTROKE: f32 = 0.4;
/// The outer wing (the hand and the primaries) trails the inner one by
/// this share of a beat.
pub const WRIST_LAG: f32 = 0.1;
/// The braking flaps when landing last this long (s); then the wings fold.
pub const BRAKE_S: f32 = 0.9;
/// Flap-gliding while cruising level: a burst of 3 … 5 beats, then a glide
/// of 0.6 … 1.4 s with the wings spread.
pub const BURST_BEATS: (u8, u8) = (3, 5);
pub const GLIDE_S: (f32, f32) = (0.6, 1.4);

/// The wing's height in its beat (+1 top … −1 bottom) at `phase` (0 … 1,
/// 0 = the top): an eased downstroke over [`DOWNSTROKE`] of the beat, the
/// slower upstroke over the rest.
pub fn stroke(phase: f32) -> f32 {
    let p = phase.rem_euclid(1.0);
    let smooth = |u: f32| u * u * (3.0 - 2.0 * u);
    if p < DOWNSTROKE {
        1.0 - 2.0 * smooth(p / DOWNSTROKE)
    } else {
        -1.0 + 2.0 * smooth((p - DOWNSTROKE) / (1.0 - DOWNSTROKE))
    }
}

/// How far into the upstroke the wing is (0 on the downstroke, rising to 1
/// in the middle of the upstroke and back): the wing folds and sweeps
/// back, the primaries close.
pub fn upstroke(phase: f32) -> f32 {
    let p = phase.rem_euclid(1.0);
    if p < DOWNSTROKE {
        0.0
    } else {
        (PI * (p - DOWNSTROKE) / (1.0 - DOWNSTROKE)).sin()
    }
}

/// A number from `seed` and `n` (deterministic: the same rider flaps the
/// same way on every client).
fn mix(seed: u32, n: u32) -> u32 {
    let mut x = seed ^ n.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

/// The animation state of one unicorn (pure; advanced each frame).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnimState {
    /// Gallop cycle (0 … 1, wraps).
    pub gallop: f32,
    /// Wing beat cycle (0 … 1, wraps; 0 = wings up).
    pub wing: f32,
    /// Wing beat depth (smoothed; 0 = gliding, ~1 a full beat).
    pub flap: f32,
    /// Wing spread (smoothed; 0 folded … 1 wide).
    pub spread: f32,
    /// Body bank and pitch (smoothed, rad).
    pub bank: f32,
    pub pitch: f32,
    /// Time (s), for the mane and the tail.
    pub time: f32,
    /// From the rider id: the beat's phase and the flap-glide rhythm.
    pub seed: Option<u32>,
    /// Beats left in this burst, the glide's time left (s), bursts so far.
    pub beats_left: u8,
    pub glide_left: f32,
    pub burst: u32,
    /// Seconds since landing began (the braking flaps).
    pub landing_s: f32,
}

fn approach(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    current + (target - current) * (1.0 - (-dt / tau).exp())
}

impl AnimState {
    /// Whether the wings are held still in a glide between bursts.
    pub fn gliding(&self) -> bool {
        self.glide_left > 0.0
    }

    /// Advances by `dt` seconds of the flight `v`.
    pub fn advance(&mut self, v: &UnicornView, dt: f32) {
        use ocw_sim::god;
        let seed = match self.seed {
            Some(s) => s,
            None => {
                // Each rider its own phase and rhythm.
                let s = mix(v.id.0, 0x0C0F_FEE5);
                self.seed = Some(s);
                self.wing = (s % 997) as f32 / 997.0;
                self.beats_left = BURST_BEATS.0 + ((s >> 12) % 3) as u8;
                s
            }
        };
        let speed = v.speed_mps;
        self.gallop = (self.gallop + god::gallop_hz(speed) as f32 * dt).fract();
        self.landing_s = if v.landing { self.landing_s + dt } else { 0.0 };
        let braking = v.landing && self.landing_s < BRAKE_S;
        let (hz, depth) = if braking {
            (2.4, 1.3)
        } else if v.landing {
            (1.5, 0.0)
        } else {
            (
                god::wing_beat_hz(v.climb_mps, v.accel, speed) as f32,
                god::wing_beat_depth(v.climb_mps, v.accel, speed) as f32,
            )
        };
        // Flap-gliding only while cruising level (climbing, speeding up
        // and landing flap on).
        let may_glide = !v.landing && v.climb_mps < 2.0 && v.accel < 5.0 && speed > 8.0;
        if !may_glide {
            self.glide_left = 0.0;
        } else if self.glide_left > 0.0 {
            self.glide_left = (self.glide_left - dt).max(0.0);
        }
        let beating = hz > 0.0 && !self.gliding();
        // Eased in and out (no popping), settling a little quicker.
        let (goal, tau) = if beating { (depth, 0.25) } else { (0.0, 0.18) };
        self.flap = approach(self.flap, goal, dt, tau);
        let before = self.wing;
        self.wing = (self.wing + hz.max(1.2) * dt).fract();
        if beating && self.wing < before {
            // A beat done (at the top of the stroke).
            self.beats_left = self.beats_left.saturating_sub(1);
            if self.beats_left == 0 {
                self.burst += 1;
                let h = mix(seed, self.burst);
                self.beats_left = BURST_BEATS.0 + (h % 3) as u8;
                if may_glide {
                    let u = ((h >> 8) % 1000) as f32 / 1000.0;
                    self.glide_left = GLIDE_S.0 + (GLIDE_S.1 - GLIDE_S.0) * u;
                }
            }
        }
        let spread = if braking {
            1.0
        } else if v.landing {
            0.0
        } else {
            god::wing_spread(speed, v.turn_rate) as f32
        };
        self.spread = approach(self.spread, spread, dt, 0.35);
        self.bank = approach(self.bank, god::bank(v.turn_rate, speed) as f32, dt, 0.3);
        let pitch = if speed > 1.0 {
            (v.climb_mps / speed.max(10.0)).clamp(-0.35, 0.35) as f32
        } else {
            0.0
        };
        self.pitch = approach(self.pitch, pitch, dt, 0.4);
        self.time += dt;
    }
}

/// One wing's angles (rad) at a moment of the beat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WingAngles {
    /// The shoulder's flap (tips up +) and sweep (back +).
    pub shoulder: f32,
    pub sweep: f32,
    /// The wrist's flap and sweep relative to the arm: the hand trails.
    pub wrist: f32,
    pub wrist_sweep: f32,
    /// The primaries' fan (1 spread … 0 closed).
    pub fan: f32,
}

/// Limits of the wing joints (rad) that [`wing_angles`] stays within.
pub const SHOULDER_LIMIT: f32 = 1.25;
pub const WRIST_LIMIT: f32 = 1.1;
pub const SWEEP_LIMITS: (f32, f32) = (-0.45, 1.75);

/// The angles of a wing (`side` 0 left, 1 right) for the state `a`: the
/// shoulder beats with [`stroke`], the hand follows [`WRIST_LAG`] later
/// and a little deeper, the upstroke folds and sweeps it back and closes
/// the primaries; folded along the body when the spread is 0; in a bank
/// the inner (lower) wing folds a little and the outer one stretches; the
/// braking flaps of landing reach forward.
pub fn wing_angles(a: &AnimState, side: u8) -> WingAngles {
    let depth = a.flap;
    let s_in = stroke(a.wing);
    let s_out = stroke(a.wing - WRIST_LAG);
    let up = upstroke(a.wing) * depth.min(1.0);
    let fold = 1.0 - a.spread;
    // A right bank (positive) puts the right wing inside the turn.
    let inner = if side == 1 { a.bank } else { -a.bank };
    let tuck = inner.max(0.0) * 0.5 - (-inner).max(0.0) * 0.15;
    let braking = a.landing_s > 0.0 && a.landing_s < BRAKE_S;
    let shoulder = depth * 0.7 * s_in + 0.1 * a.spread - 0.2 * fold;
    let outer = depth * 0.9 * s_out;
    let mut sweep = fold * 1.35 + 0.22 * up + 0.4 * tuck;
    if braking {
        // Reaching forward to brake.
        sweep -= 0.35 * (1.0 - up);
    }
    WingAngles {
        shoulder: shoulder.clamp(-SHOULDER_LIMIT, SHOULDER_LIMIT),
        sweep: sweep.clamp(SWEEP_LIMITS.0, SWEEP_LIMITS.1),
        wrist: (outer - depth * 0.7 * s_in).clamp(-WRIST_LIMIT, WRIST_LIMIT),
        wrist_sweep: (0.55 * up + 0.3 * fold + tuck).clamp(SWEEP_LIMITS.0, SWEEP_LIMITS.1),
        fan: ((1.0 - 0.85 * upstroke(a.wing) * depth.min(1.0)) * a.spread).clamp(0.0, 1.0),
    }
}

/// The pose: each joint's rotation, the body's bob (m) and its tilt.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub joints: BTreeMap<Joint, Quat>,
    pub bob: f32,
    /// The body's pitch (nose up, rad) and bank (right wing down, rad).
    pub pitch: f32,
    pub bank: f32,
    /// The shoulder's flap angle (rad, tips up) and the spread (0 … 1),
    /// for tests.
    pub flap: f32,
    pub spread: f32,
}

/// The unicorn's pose from its animation state (pure).
pub fn pose(a: &AnimState, v: &UnicornView) -> Pose {
    let speed = v.speed_mps;
    let tucked = ocw_sim::god::legs_tucked(speed) as f32;
    let hovering = (1.0 - speed as f32 / 8.0).clamp(0.0, 1.0);
    let wind = (speed as f32 / 60.0).clamp(0.0, 1.0);
    let mut joints = BTreeMap::new();
    let g = TAU * a.gallop;
    // The beat moves the body: up on the downstroke (a little after it),
    // the neck and head counter-moving, the tail answering.
    let lift = -stroke(a.wing - 0.2) * a.flap;
    joints.insert(
        Joint::Neck,
        Quat::from_rotation_x(-0.6 + 0.05 * (2.0 * g).sin() - 0.07 * lift),
    );
    joints.insert(Joint::Head, Quat::from_rotation_x(0.22 + 0.05 * lift));
    // A rotary gallop: the legs one after the other; a slow paddle while
    // hovering; folded in at speed.
    let amp = 0.55 - 0.3 * hovering;
    for leg in 0..4u8 {
        let offset = [0.0, 0.1, 0.5, 0.6][leg as usize] * TAU;
        let s = (g + offset).sin();
        let front = leg < 2;
        let (hip, knee) = if front {
            (amp * s, -(0.9 * (g + offset + 0.8).sin()).max(0.0))
        } else {
            (amp * s, (0.7 * (g + offset - 0.6).sin()).max(0.0))
        };
        let (tuck_hip, tuck_knee) = if front { (1.0, -1.7) } else { (-0.9, 1.3) };
        let hip = hip + (tuck_hip - hip) * tucked;
        let knee = knee + (tuck_knee - knee) * tucked;
        joints.insert(Joint::Hip(leg), Quat::from_rotation_x(hip));
        joints.insert(Joint::Knee(leg), Quat::from_rotation_x(knee));
    }
    // Wings: shoulder, wrist and the primaries' fan.
    let mut flap = 0.0;
    for side in 0..2u8 {
        let sx = if side == 0 { -1.0 } else { 1.0 };
        let w = wing_angles(a, side);
        flap = w.shoulder;
        joints.insert(
            Joint::Wing(side),
            Quat::from_rotation_y(-sx * w.sweep) * Quat::from_rotation_z(sx * w.shoulder),
        );
        joints.insert(
            Joint::WingTip(side),
            Quat::from_rotation_y(-sx * w.wrist_sweep) * Quat::from_rotation_z(sx * w.wrist),
        );
        for k in 0..PRIMARIES {
            // Closed: the outer feathers swing back over the inner ones.
            let close = (1.0 - w.fan) * 0.16 * f32::from(k + 1);
            joints.insert(Joint::Primary(side, k), Quat::from_rotation_y(-sx * close));
        }
    }
    for i in 0..STRANDS {
        let k = f32::from(i);
        let wave = (a.time * (4.0 + 6.0 * wind) + k * 0.8).sin();
        joints.insert(
            Joint::Mane(i),
            Quat::from_rotation_x(0.25 + 0.7 * wind + 0.18 * wave),
        );
        joints.insert(
            Joint::Tail(i),
            Quat::from_rotation_x(1.9 - 0.35 * wind + 0.12 * wave + 0.1 * lift)
                * Quat::from_rotation_z((k - 3.5) * 0.07 * (1.0 + wind)),
        );
    }
    Pose {
        joints,
        bob: 0.06 * (2.0 * g).sin() * (1.0 - tucked)
            + 0.08 * hovering * (a.time * 1.6).sin()
            + 0.07 * lift,
        pitch: a.pitch,
        bank: a.bank,
        flap,
        spread: a.spread,
    }
}

/// The growth of the transformation (0 … 1 over 0.6 s, with a little
/// overshoot).
pub fn appear_scale(age_s: f64) -> f32 {
    let t = (age_s / ocw_sim::god::TRANSFORM_S).clamp(0.0, 1.0) as f32;
    let c = 1.7;
    let u = t - 1.0;
    (1.0 + (c + 1.0) * u * u * u + c * u * u).max(0.0)
}

/// The unicorn's root transform.
pub fn root_transform(v: &UnicornView, p: &Pose) -> Transform {
    Transform {
        translation: to_vec3(v.pos) + Vec3::Y * p.bob,
        rotation: Quat::from_rotation_y(v.yaw as f32)
            * Quat::from_rotation_x(p.pitch)
            * Quat::from_rotation_z(-p.bank),
        scale: Vec3::splat(appear_scale(v.age_s)),
    }
}

// ---------------------------------------------------------------- Bevy

#[derive(Component)]
struct UnicornRoot {
    id: RiderId,
}

#[derive(Component)]
struct UnicornRig {
    joints: BTreeMap<Joint, Entity>,
    anim: AnimState,
}

#[derive(Component)]
struct Sparkle {
    velocity: Vec3,
    age: f32,
    life: f32,
    size: f32,
}

#[derive(Component)]
struct Trail {
    id: RiderId,
    points: VecDeque<(f32, Vec3)>,
    mesh: Handle<Mesh>,
}

#[derive(Component)]
struct LandingRing;

/// The camera-facing card of a far unicorn ([`Lod::Impostor`]).
#[derive(Component)]
struct Impostor {
    id: RiderId,
}

/// The impostor's card (a quad 1 m high, its picture's aspect) and its
/// unlit, alpha-masked material.
fn impostor_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let (w, h) = IMPOSTOR_PX;
    let data: Vec<u8> = impostor_image().into_iter().flatten().collect();
    let image = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    let mesh = meshes.add(Rectangle::new(w as f32 / h as f32, 1.0));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(images.add(image)),
        alpha_mode: AlphaMode::Mask(0.5),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    (mesh, material)
}

/// The impostor's height (m) at `distance_m` from the camera: the
/// unicorn's size (2.6 m), growing far away so it stays a few pixels
/// tall (a beacon of the rider in the sky).
pub fn impostor_height(distance_m: f32) -> f32 {
    (distance_m * 0.006).max(2.6)
}

/// Turns the impostor cards to the camera, at their unicorn, flipped so
/// the head points the way it flies (seen from the camera).
fn impostors(
    view: Res<RideView>,
    cameras: Query<&GlobalTransform, With<FollowCamera>>,
    mut cards: Query<(&Impostor, &mut Transform)>,
) {
    let Some(cam) = cameras.iter().next() else {
        return;
    };
    let eye = cam.translation();
    for (card, mut t) in &mut cards {
        let Some(v) = view.unicorns.iter().find(|v| v.id == card.id) else {
            continue;
        };
        let at = to_vec3(v.pos);
        let to_cam = (eye - at).with_y(0.0).normalize_or(Vec3::Z);
        let h = impostor_height(eye.distance(at));
        // The flight direction seen from the camera: left or right.
        let right = Vec3::Y.cross(to_cam);
        let heading = crate::camera::view_direction(v.yaw, 0.0);
        let flip = if heading.dot(right) < 0.0 { -1.0 } else { 1.0 };
        *t = Transform::from_translation(at)
            .looking_to(-to_cam, Vec3::Y)
            .with_scale(Vec3::new(flip * h, h, h));
    }
}

#[derive(Component)]
struct NameTag;

/// The unicorns' entities and the shared assets.
#[derive(Resource, Default)]
struct Unicorns {
    roots: BTreeMap<RiderId, (Entity, Vec3)>,
    trails: BTreeMap<RiderId, Entity>,
    tags: BTreeMap<RiderId, Entity>,
    meshes: Vec<(Joint, Paint, Lod, Handle<Mesh>)>,
    /// The impostor card of each unicorn, its mesh and material.
    impostors: BTreeMap<RiderId, Entity>,
    impostor: Option<(Handle<Mesh>, Handle<StandardMaterial>)>,
    materials: BTreeMap<(Paint, u32), Handle<StandardMaterial>>,
    sparkle_mesh: Option<Handle<Mesh>>,
    sparkle_materials: Vec<Handle<StandardMaterial>>,
    ring: Option<Entity>,
    seed: u32,
}

fn to_mesh(shape: &Shape) -> Mesh {
    let (pos, nor) = shape.flat();
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    m
}

fn paint_material(paint: Paint, rider: u32) -> StandardMaterial {
    let glow = |c: Color, k: f32| {
        let l = c.to_linear();
        LinearRgba::rgb(l.red * k, l.green * k, l.blue * k)
    };
    match paint {
        Paint::Coat => StandardMaterial {
            base_color: Color::srgb(0.97, 0.96, 0.98),
            perceptual_roughness: 0.55,
            ..default()
        },
        Paint::Feather => StandardMaterial {
            base_color: Color::srgb(0.99, 0.97, 1.0),
            perceptual_roughness: 0.45,
            emissive: LinearRgba::rgb(0.05, 0.05, 0.07),
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        Paint::Gold => StandardMaterial {
            base_color: Color::srgb(1.0, 0.78, 0.25),
            metallic: 0.85,
            perceptual_roughness: 0.28,
            emissive: LinearRgba::rgb(0.12, 0.08, 0.01),
            ..default()
        },
        Paint::Eye => StandardMaterial {
            base_color: Color::srgb(0.08, 0.06, 0.12),
            perceptual_roughness: 0.15,
            ..default()
        },
        Paint::Muzzle => StandardMaterial {
            base_color: Color::srgb(0.96, 0.86, 0.9),
            perceptual_roughness: 0.6,
            ..default()
        },
        Paint::Rider => StandardMaterial {
            base_color: srgb(rider),
            emissive: glow(srgb(rider), 0.12),
            perceptual_roughness: 0.5,
            ..default()
        },
        Paint::Band(i) => {
            let c = srgb(RAINBOW[usize::from(i) % RAINBOW.len()]);
            StandardMaterial {
                base_color: c,
                emissive: glow(c, 0.25),
                perceptual_roughness: 0.5,
                ..default()
            }
        }
    }
}

/// A small deterministic random number (0 … 1) for the sparkles.
fn rand01(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed % 10_000) as f32 / 10_000.0
}

fn burst(
    commands: &mut Commands,
    u: &mut Unicorns,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
) {
    let mesh = u
        .sparkle_mesh
        .get_or_insert_with(|| meshes.add(to_mesh(&ellipsoid(Vec3::splat(1.0), 5, 3))))
        .clone();
    if u.sparkle_materials.is_empty() {
        let mut cols: Vec<u32> = RAINBOW.to_vec();
        cols.push(0xFFF4C0);
        cols.push(0xFFFFFF);
        u.sparkle_materials = cols
            .into_iter()
            .map(|c| {
                let l = srgb(c).to_linear();
                materials.add(StandardMaterial {
                    base_color: srgb(c),
                    emissive: LinearRgba::rgb(l.red * 4.0, l.green * 4.0, l.blue * 4.0),
                    unlit: true,
                    ..default()
                })
            })
            .collect();
    }
    if u.seed == 0 {
        u.seed = 0x9E37_79B9;
    }
    for k in 0..48 {
        let mut s = u.seed;
        let (a, b, c, d) = (
            rand01(&mut s),
            rand01(&mut s),
            rand01(&mut s),
            rand01(&mut s),
        );
        u.seed = s;
        let dir = Vec3::new(a * 2.0 - 1.0, b * 1.6 - 0.4, c * 2.0 - 1.0).normalize_or_zero();
        let material = u.sparkle_materials[k % u.sparkle_materials.len()].clone();
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(at + dir * 0.4).with_scale(Vec3::splat(0.06)),
            bevy::light::NotShadowCaster,
            Sparkle {
                velocity: dir * (2.5 + 4.0 * d) + Vec3::Y * 1.2,
                age: 0.0,
                life: 0.55 + 0.4 * d,
                size: 0.05 + 0.06 * a,
            },
        ));
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn sync_unicorns(
    mut commands: Commands,
    view: Res<RideView>,
    mut u: ResMut<Unicorns>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: Option<ResMut<Assets<Image>>>,
    roots: Query<(), With<UnicornRoot>>,
) {
    let u = &mut *u;
    if u.meshes.is_empty() {
        u.meshes = [Lod::Near, Lod::Far]
            .into_iter()
            .flat_map(|lod| {
                model_lod(lod)
                    .into_iter()
                    .map(move |p| (p.joint, p.paint, lod, p.shape))
            })
            .map(|(j, p, lod, shape)| (j, p, lod, meshes.add(to_mesh(&shape))))
            .collect();
    }
    if u.impostor.is_none()
        && let Some(images) = images.as_deref_mut()
    {
        u.impostor = Some(impostor_assets(&mut meshes, &mut materials, images));
    }
    // Gone: back into the rider, with a sparkle burst.
    let present: Vec<RiderId> = view.unicorns.iter().map(|v| v.id).collect();
    let gone: Vec<RiderId> = u
        .roots
        .keys()
        .copied()
        .filter(|id| !present.contains(id))
        .collect();
    for id in gone {
        let (e, at) = u.roots.remove(&id).expect("listed");
        if roots.contains(e) {
            commands.entity(e).despawn();
        }
        if let Some(i) = u.impostors.remove(&id) {
            commands.entity(i).despawn();
        }
        burst(&mut commands, u, &mut meshes, &mut materials, at);
    }
    for v in &view.unicorns {
        if let Some((e, last)) = u.roots.get_mut(&v.id)
            && roots.contains(*e)
        {
            *last = to_vec3(v.pos);
            continue;
        }
        let rider = v.colour.0 & 0xFF_FFFF;
        let skel = skeleton();
        let root = commands
            .spawn((
                Transform::from_translation(to_vec3(v.pos)).with_scale(Vec3::splat(0.01)),
                Visibility::default(),
                UnicornRoot { id: v.id },
            ))
            .id();
        let mut joints = BTreeMap::new();
        joints.insert(Joint::Body, root);
        for d in &skel {
            let Some(parent) = d.parent else { continue };
            let p = joints[&parent];
            let e = commands
                .spawn((
                    Transform::from_translation(d.pivot),
                    Visibility::default(),
                    ChildOf(p),
                ))
                .id();
            joints.insert(d.joint, e);
        }
        for (joint, paint, lod, mesh) in &u.meshes {
            let key = (*paint, if *paint == Paint::Rider { rider } else { 0 });
            let material = u
                .materials
                .entry(key)
                .or_insert_with(|| materials.add(paint_material(*paint, rider)))
                .clone();
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                Transform::default(),
                lod.range(),
                ChildOf(joints[joint]),
            ));
        }
        if let Some((mesh, material)) = &u.impostor {
            let card = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(to_vec3(v.pos)),
                    Lod::Impostor.range(),
                    bevy::light::NotShadowCaster,
                    Impostor { id: v.id },
                ))
                .id();
            if let Some(old) = u.impostors.insert(v.id, card) {
                commands.entity(old).despawn();
            }
        }
        commands.entity(root).insert(UnicornRig {
            joints,
            anim: AnimState::default(),
        });
        u.roots.insert(v.id, (root, to_vec3(v.pos)));
        if v.age_s < ocw_sim::god::TRANSFORM_S {
            burst(
                &mut commands,
                u,
                &mut meshes,
                &mut materials,
                to_vec3(v.pos),
            );
        }
    }
}

fn animate(
    time: Res<Time>,
    view: Res<RideView>,
    mut rigs: Query<(&UnicornRoot, &mut UnicornRig)>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    for (root, mut rig) in &mut rigs {
        let Some(v) = view.unicorns.iter().find(|v| v.id == root.id) else {
            continue;
        };
        rig.anim.advance(v, dt);
        let p = pose(&rig.anim, v);
        if let Ok(mut t) = transforms.get_mut(rig.joints[&Joint::Body]) {
            *t = root_transform(v, &p);
        }
        for (joint, q) in &p.joints {
            if let Some(e) = rig.joints.get(joint)
                && let Ok(mut t) = transforms.get_mut(*e)
            {
                t.rotation = *q;
            }
        }
    }
}

fn sparkles(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Sparkle, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut s, mut t) in &mut q {
        s.age += dt;
        if s.age >= s.life {
            commands.entity(e).despawn();
            continue;
        }
        s.velocity *= (-dt * 2.0).exp();
        t.translation += s.velocity * dt;
        let k = 1.0 - s.age / s.life;
        t.scale = Vec3::splat(s.size * (0.3 + k) * (1.0 + 0.4 * (s.age * 30.0).sin()));
    }
}

/// Trail length (s).
pub const TRAIL_S: f32 = 2.0;
/// Width of one rainbow band of the trail (m).
const TRAIL_BAND_M: f32 = 0.09;

/// The trail ribbon from `points` (time, position; oldest first) at time
/// `now`: eight bands, fading out over [`TRAIL_S`]. Pure (positions,
/// colours, indices).
pub fn trail_mesh(
    points: &VecDeque<(f32, Vec3)>,
    now: f32,
) -> (Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<u32>) {
    let mut pos = Vec::new();
    let mut col = Vec::new();
    let mut idx = Vec::new();
    if points.len() < 2 {
        return (pos, col, idx);
    }
    let bands = RAINBOW.len();
    let half = TRAIL_BAND_M * bands as f32 / 2.0;
    for (k, &(t, p)) in points.iter().enumerate() {
        let next = points
            .get(k + 1)
            .map_or_else(|| p * 2.0 - points[k - 1].1, |n| n.1);
        let prev = if k > 0 { points[k - 1].1 } else { p };
        let dir = (next - prev).normalize_or_zero();
        let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
        let fade = (1.0 - (now - t) / TRAIL_S).clamp(0.0, 1.0);
        // Thin at the tail end.
        let w = 0.35 + 0.65 * fade;
        for b in 0..=bands {
            let off = (-half + TRAIL_BAND_M * b as f32) * w;
            pos.push((p + side * off).to_array());
            let c = srgb(RAINBOW[b.min(bands - 1)]).to_linear();
            col.push([c.red, c.green, c.blue, 0.32 * fade * fade]);
        }
    }
    let row = (bands + 1) as u32;
    for k in 0..points.len() as u32 - 1 {
        for b in 0..bands as u32 {
            let (a, c) = (k * row + b, (k + 1) * row + b);
            idx.extend([a, c, a + 1, a + 1, c, c + 1]);
        }
    }
    (pos, col, idx)
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn trails(
    mut commands: Commands,
    time: Res<Time>,
    view: Res<RideView>,
    on: Res<UnicornTrail>,
    mut u: ResMut<Unicorns>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    roots: Query<&Transform, (With<UnicornRoot>, Without<Trail>)>,
    mut q: Query<(Entity, &mut Trail, &mut Visibility)>,
) {
    let now = time.elapsed_secs();
    let u = &mut *u;
    for v in &view.unicorns {
        if u.trails.contains_key(&v.id) {
            continue;
        }
        let mesh = meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        ));
        let material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        let e = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                Transform::default(),
                Visibility::Hidden,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
                Trail {
                    id: v.id,
                    points: VecDeque::new(),
                    mesh,
                },
            ))
            .id();
        u.trails.insert(v.id, e);
    }
    for (e, mut trail, mut vis) in &mut q {
        let Some(v) = view.unicorns.iter().find(|v| v.id == trail.id) else {
            commands.entity(e).despawn();
            u.trails.remove(&trail.id);
            continue;
        };
        let tail = u
            .roots
            .get(&v.id)
            .and_then(|(r, _)| roots.get(*r).ok())
            .map_or(to_vec3(v.pos), |t| {
                t.transform_point(Vec3::new(0.0, 0.15, 1.5))
            });
        let fresh = trail
            .points
            .back()
            .is_none_or(|(t, p)| now - t >= 0.04 || p.distance(tail) > 30.0);
        if fresh {
            trail.points.push_back((now, tail));
        } else if let Some(last) = trail.points.back_mut() {
            last.1 = tail;
        }
        while trail.points.front().is_some_and(|(t, _)| now - t > TRAIL_S) {
            trail.points.pop_front();
        }
        // A jump (teleport): start again.
        if trail
            .points
            .iter()
            .zip(trail.points.iter().skip(1))
            .any(|(a, b)| a.1.distance(b.1) > 200.0)
        {
            trail.points.retain(|(t, _)| *t == now);
        }
        let show = on.0 && trail.points.len() >= 2;
        *vis = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if !show {
            continue;
        }
        let (pos, col, idx) = trail_mesh(&trail.points, now);
        if let Some(mut m) = meshes.get_mut(&trail.mesh) {
            let n = pos.len();
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; n]);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
            m.insert_indices(Indices::U32(idx));
        }
    }
}

/// The landing ring's mesh: a flat torus of radius `r` (m) and tube
/// radius `t`, lying in the ground plane.
pub fn ring_shape(r: f32, t: f32) -> Shape {
    const MAJOR: u32 = 32;
    const MINOR: u32 = 6;
    let mut positions = Vec::new();
    for i in 0..MAJOR {
        let u = TAU * i as f32 / MAJOR as f32;
        for j in 0..MINOR {
            let w = TAU * j as f32 / MINOR as f32;
            let rr = r + t * w.cos();
            positions.push(Vec3::new(rr * u.cos(), t * w.sin(), rr * u.sin()));
        }
    }
    let at = |i: u32, j: u32| (i % MAJOR) * MINOR + j % MINOR;
    let mut indices = Vec::new();
    for i in 0..MAJOR {
        for j in 0..MINOR {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
            indices.extend([a, c, b, b, c, d]);
        }
    }
    Shape { positions, indices }
}

fn landing_ring(
    mut commands: Commands,
    time: Res<Time>,
    view: Res<RideView>,
    mut u: ResMut<Unicorns>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut rings: Query<(&mut Transform, &mut Visibility), With<LandingRing>>,
) {
    let spot = view
        .flight
        .as_ref()
        .and_then(|f| f.landing.filter(|_| !f.leaving));
    let ring = match u.ring.filter(|e| rings.contains(*e)) {
        Some(e) => e,
        None => {
            if spot.is_none() {
                return;
            }
            let e = commands
                .spawn((
                    Mesh3d(meshes.add(to_mesh(&ring_shape(2.2, 0.16)))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgb(1.0, 0.9, 0.5),
                        emissive: LinearRgba::rgb(6.0, 4.4, 1.6),
                        unlit: true,
                        ..default()
                    })),
                    Transform::default(),
                    Visibility::Hidden,
                    bevy::light::NotShadowCaster,
                    LandingRing,
                ))
                .id();
            u.ring = Some(e);
            return;
        }
    };
    let Ok((mut t, mut vis)) = rings.get_mut(ring) else {
        return;
    };
    match spot {
        Some(p) => {
            let pulse = 1.0 + 0.12 * (time.elapsed_secs() * 4.0).sin();
            *t = Transform::from_translation(to_vec3(p) + Vec3::Y * 0.15)
                .with_scale(Vec3::new(pulse, 1.0, pulse));
            *vis = Visibility::Visible;
        }
        None => *vis = Visibility::Hidden,
    }
}

/// Name tags over the other riders' unicorns (up to 3 km away).
fn name_tags(
    mut commands: Commands,
    view: Res<RideView>,
    mut u: ResMut<Unicorns>,
    cameras: Query<(&Camera, &GlobalTransform), With<FollowCamera>>,
    mut tags: Query<(&mut Node, &mut Visibility, &mut Text), With<NameTag>>,
) {
    let others: Vec<&UnicornView> = view.unicorns.iter().filter(|v| !v.is_local).collect();
    for v in &others {
        if u.tags.contains_key(&v.id) {
            continue;
        }
        let e = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    padding: UiRect::axes(px(6), px(2)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                Text::new(v.name.clone()),
                TextFont {
                    font_size: 15.0.into(),
                    ..default()
                },
                TextColor(Color::WHITE),
                Visibility::Hidden,
                NameTag,
            ))
            .id();
        u.tags.insert(v.id, e);
    }
    let cam = cameras.iter().next();
    let mut gone = Vec::new();
    for (&id, &e) in &u.tags {
        let Ok((mut node, mut vis, mut text)) = tags.get_mut(e) else {
            continue;
        };
        let Some(v) = others.iter().find(|v| v.id == id) else {
            commands.entity(e).despawn();
            gone.push(id);
            continue;
        };
        if text.0 != v.name {
            text.0 = v.name.clone();
        }
        let head = to_vec3(v.pos) + Vec3::Y * 2.4;
        let shown = cam.and_then(|(c, g)| {
            (g.translation().distance(head) < 3_000.0)
                .then(|| c.world_to_viewport(g, head).ok())
                .flatten()
        });
        match shown {
            Some(p) => {
                node.left = px(p.x - 30.0);
                node.top = px(p.y - 24.0);
                *vis = Visibility::Visible;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    for id in gone {
        u.tags.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(speed: f64) -> UnicornView {
        UnicornView {
            id: RiderId(1),
            name: "Uni".into(),
            is_local: true,
            pos: [0.0, 100.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            speed_mps: speed,
            climb_mps: 0.0,
            turn_rate: 0.0,
            accel: 0.0,
            age_s: 5.0,
            landing: false,
            colour: ocw_core::Rgb(0xFF1472),
        }
    }

    fn run(v: &UnicornView, seconds: f32) -> AnimState {
        let mut a = AnimState::default();
        for _ in 0..(seconds * 60.0) as usize {
            a.advance(v, 1.0 / 60.0);
        }
        a
    }

    #[test]
    fn the_model_is_low_poly_and_complete() {
        let parts = model();
        let tris: usize = parts.iter().map(|p| p.shape.triangles()).sum();
        assert!((1_500..=3_500).contains(&tris), "{tris} triangles");
        let skel = skeleton();
        // Every part hangs on a joint of the skeleton; parents come first.
        for p in &parts {
            assert!(skel.iter().any(|d| d.joint == p.joint), "{:?}", p.joint);
        }
        for (i, d) in skel.iter().enumerate() {
            if let Some(parent) = d.parent {
                assert!(skel[..i].iter().any(|e| e.joint == parent));
            }
        }
        // A golden horn, the rider's colour once (the first mane band; no
        // saddle cloth on the back since 2026-11-13, in no LOD), the
        // rainbow in mane and tail, two wings of feathers.
        assert!(
            parts
                .iter()
                .any(|p| p.joint == Joint::Head && p.paint == Paint::Gold)
        );
        for lod in [Lod::Near, Lod::Far] {
            let parts = model_lod(lod);
            let rider: Vec<_> = parts.iter().filter(|p| p.paint == Paint::Rider).collect();
            assert_eq!(rider.len(), 1, "{lod:?}");
            assert_eq!(rider[0].joint, Joint::Mane(0), "{lod:?}");
            // Nothing but the coat on the back (the saddle cloth's slabs).
            assert!(
                parts
                    .iter()
                    .all(|p| p.joint != Joint::Body || p.paint == Paint::Coat),
                "{lod:?}"
            );
        }
        assert_eq!(
            parts
                .iter()
                .filter(|p| matches!(p.joint, Joint::Tail(_)) && matches!(p.paint, Paint::Band(_)))
                .count(),
            8
        );
        // A wing: the arm (covert and inner feathers), the hand's covert,
        // the primaries in their groups – on both LODs.
        let left = |p: &&Part| {
            matches!(
                p.joint,
                Joint::Wing(0) | Joint::WingTip(0) | Joint::Primary(0, _)
            )
        };
        assert!(parts.iter().filter(left).count() >= 14);
        for lod in [Lod::Near, Lod::Far] {
            let m = model_lod(lod);
            assert!(m.iter().any(|p| p.joint == Joint::WingTip(1)), "{lod:?}");
            for k in 0..PRIMARIES {
                assert!(
                    m.iter().any(|p| p.joint == Joint::Primary(1, k)),
                    "{lod:?} {k}"
                );
            }
        }
        // Flat normals point outwards on a closed shape.
        let (pos, nor) = ellipsoid(Vec3::new(1.0, 2.0, 3.0), 8, 6).flat();
        for (p, n) in pos.iter().zip(&nor) {
            assert!(Vec3::from(*p).dot(Vec3::from(*n)) > 0.0);
        }
        let (pos, nor) = tube(0.2, 0.1, 1.0, 6).flat();
        for (p, n) in pos.iter().zip(&nor) {
            let c = Vec3::new(0.0, 0.5, 0.0);
            assert!((Vec3::from(*p) - c).dot(Vec3::from(*n)) > -1e-6);
        }
    }

    /// LOD (spec §14 Phase 13C): about 3k triangles near, ~600 far, an
    /// impostor card beyond 500 m.
    #[test]
    fn the_levels_of_detail_and_the_impostor() {
        let tris = |lod| -> usize { model_lod(lod).iter().map(|p| p.shape.triangles()).sum() };
        let (near, far) = (tris(Lod::Near), tris(Lod::Far));
        assert!((2_500..=3_500).contains(&near), "near {near}");
        assert!((450..=800).contains(&far), "far {far}");
        assert_eq!(tris(Lod::Impostor), 0);
        // The far model keeps every joint moving (wings, mane, tail, legs).
        let joints = |lod| {
            let mut j: Vec<Joint> = model_lod(lod).iter().map(|p| p.joint).collect();
            j.sort();
            j.dedup();
            j
        };
        assert_eq!(joints(Lod::Near), joints(Lod::Far));
        assert_eq!(Lod::at(8.0), Lod::Near);
        assert_eq!(Lod::at(79.0), Lod::Near);
        assert_eq!(Lod::at(120.0), Lod::Far);
        assert_eq!(Lod::at(499.0), Lod::Far);
        assert_eq!(Lod::at(501.0), Lod::Impostor);
        assert_eq!(Lod::at(5_000.0), Lod::Impostor);
        // The ranges meet: where one fades out the next fades in.
        assert_eq!(Lod::Near.range().end_margin, Lod::Far.range().start_margin);
        assert!(Lod::Far.range().end_margin.contains(&FAR_END_M));
        // The card: a white unicorn on a transparent ground, rainbow tail.
        let img = impostor_image();
        let (w, h) = IMPOSTOR_PX;
        assert_eq!(img.len(), (w * h) as usize);
        assert_eq!(img[0][3], 0, "transparent corner");
        let opaque = img.iter().filter(|p| p[3] == 255).count();
        assert!(opaque > img.len() / 8 && opaque < img.len() / 2, "{opaque}");
        assert!(
            img.iter()
                .any(|p| p[3] == 255 && p[0] > 240 && p[1] > 240 && p[2] > 240)
        );
        assert!(impostor_height(10.0) >= 2.6 && impostor_height(3_000.0) > 10.0);
    }

    #[test]
    fn wings_fold_when_hovering_and_spread_in_flight() {
        let hover = run(&view(0.0), 3.0);
        assert!(hover.spread < 0.05, "{}", hover.spread);
        let fly = run(&view(80.0), 3.0);
        assert!(fly.spread > 0.7, "{}", fly.spread);
        // Landing folds them, whatever the speed.
        let landing = run(
            &UnicornView {
                landing: true,
                ..view(80.0)
            },
            3.0,
        );
        assert!(landing.spread < 0.05);
        // Climbing beats them (up to ~2 Hz), a glide down holds them still.
        let climb = UnicornView {
            climb_mps: 20.0,
            ..view(60.0)
        };
        let a = run(&climb, 3.0);
        assert!(a.flap > 0.8);
        let flaps: Vec<f32> = (0..120)
            .scan(a, |s, _| {
                s.advance(&climb, 1.0 / 60.0);
                Some(pose(s, &climb).flap)
            })
            .collect();
        let range = flaps.iter().cloned().fold(f32::MIN, f32::max)
            - flaps.iter().cloned().fold(f32::MAX, f32::min);
        assert!(range > 0.8, "beats: {range}");
        let glide = run(
            &UnicornView {
                climb_mps: -10.0,
                ..view(60.0)
            },
            3.0,
        );
        assert!(glide.flap < 0.05);
    }

    /// The flap's samples over `seconds` (60 per second) after `warm` s.
    fn trace(v: &UnicornView, warm: f32, seconds: f32) -> Vec<AnimState> {
        let mut a = run(v, warm);
        (0..(seconds * 60.0) as usize)
            .map(|_| {
                a.advance(v, 1.0 / 60.0);
                a
            })
            .collect()
    }

    /// A bird's wingbeat (spec §14 Phase 13C, 2026-11-14): the downstroke
    /// is quicker than the upstroke, the hand trails the arm.
    #[test]
    fn the_stroke_is_asymmetric_and_the_hand_trails() {
        assert_eq!(stroke(0.0), 1.0);
        assert!((stroke(DOWNSTROKE) + 1.0).abs() < 1e-6);
        assert!((stroke(1.0) - 1.0).abs() < 1e-6);
        let slope = |p: f32| (stroke(p + 1e-3) - stroke(p - 1e-3)) / 2e-3;
        let down = slope(DOWNSTROKE / 2.0).abs();
        let up = slope(DOWNSTROKE + (1.0 - DOWNSTROKE) / 2.0).abs();
        assert!(down > 1.4 * up, "down {down} up {up}");
        const { assert!(DOWNSTROKE < 0.5) };
        // Up only on the upstroke.
        assert_eq!(upstroke(0.2), 0.0);
        assert!((upstroke(0.7) - 1.0).abs() < 1e-6);
        // Over a beat the outer wing's peak comes after the arm's.
        let a = AnimState {
            flap: 1.0,
            spread: 1.0,
            ..Default::default()
        };
        let at = |p: f32| wing_angles(&AnimState { wing: p, ..a }, 1);
        let samples: Vec<(f32, f32)> = (0..200)
            .map(|i| {
                let w = at(i as f32 / 200.0);
                (w.shoulder, w.shoulder + w.wrist)
            })
            .collect();
        let argmin = |f: &dyn Fn(&(f32, f32)) -> f32| {
            (0..samples.len())
                .min_by(|&i, &j| f(&samples[i]).total_cmp(&f(&samples[j])))
                .unwrap()
        };
        let arm_low = argmin(&|s| s.0);
        let hand_low = argmin(&|s| s.1);
        assert!(hand_low > arm_low, "the hand trails: {arm_low} {hand_low}");
        // Spread on the downstroke, folded and swept back on the upstroke.
        let down = at(0.2);
        let up = at(0.7);
        assert!(down.fan > 0.95 && up.fan < 0.3, "{down:?} {up:?}");
        assert!(up.sweep > down.sweep + 0.15 && up.wrist_sweep > down.wrist_sweep + 0.4);
    }

    #[test]
    fn the_wings_beat_all_the_time_deep_when_slow_shallow_when_fast() {
        let depth = |v: &UnicornView| {
            let t = trace(v, 2.0, 6.0);
            let f: Vec<f32> = t.iter().map(|a| pose(a, v).flap).collect();
            f.iter().cloned().fold(f32::MIN, f32::max) - f.iter().cloned().fold(f32::MAX, f32::min)
        };
        let climb = |speed| UnicornView {
            climb_mps: 8.0,
            ..view(speed)
        };
        // Climbing never glides: clearly visible beats at any speed.
        let (slow, fast) = (depth(&climb(20.0)), depth(&climb(250.0)));
        assert!(slow > 1.4, "slow {slow}");
        assert!(fast > 0.8 && fast < slow, "fast {fast}");
        // The beat rate: the cycle turns faster when fast.
        let cycles = |v: &UnicornView| {
            let t = trace(v, 1.0, 5.0);
            t.windows(2).filter(|w| w[1].wing < w[0].wing).count()
        };
        assert!(cycles(&climb(250.0)) > cycles(&climb(20.0)));
        let c = cycles(&climb(60.0));
        assert!((6..=10).contains(&c), "{c} beats in 5 s");
    }

    #[test]
    fn cruising_glides_between_bursts_of_flapping() {
        let v = view(60.0);
        let t = trace(&v, 1.0, 20.0);
        let glides = t
            .windows(2)
            .filter(|w| !w[0].gliding() && w[1].gliding())
            .count();
        assert!(glides >= 3, "{glides} glides in 20 s");
        // In a glide the wings come to rest spread (eased, no jump) …
        let still = t
            .iter()
            .filter(|a| a.gliding() && a.flap < 0.05 && a.spread > 0.8)
            .count();
        assert!(still > 30, "{still}");
        // … and no frame jumps: the shoulder moves at most a little.
        let flaps: Vec<f32> = t.iter().map(|a| pose(a, &v).flap).collect();
        let jump = flaps
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(jump < 0.25, "{jump} rad in a frame");
        // Climbing or speeding up flaps on.
        let climbing = UnicornView {
            climb_mps: 10.0,
            ..view(60.0)
        };
        assert!(trace(&climbing, 1.0, 10.0).iter().all(|a| !a.gliding()));
    }

    #[test]
    fn each_rider_flaps_in_its_own_rhythm_deterministically() {
        let v1 = view(60.0);
        let v2 = UnicornView {
            id: RiderId(2),
            ..view(60.0)
        };
        assert_eq!(trace(&v1, 0.0, 5.0), trace(&v1, 0.0, 5.0));
        let (a, b) = (trace(&v1, 0.0, 3.0), trace(&v2, 0.0, 3.0));
        let same = a
            .iter()
            .zip(&b)
            .filter(|(x, y)| (x.wing - y.wing).abs() < 0.02)
            .count();
        assert!(same < a.len() / 4, "not in sync: {same}");
    }

    #[test]
    fn the_wing_joints_stay_within_their_limits() {
        let states = [
            view(0.0),
            view(15.0),
            view(400.0),
            UnicornView {
                climb_mps: 30.0,
                accel: 60.0,
                ..view(30.0)
            },
            UnicornView {
                turn_rate: 1.2,
                ..view(40.0)
            },
            UnicornView {
                landing: true,
                ..view(60.0)
            },
        ];
        for v in &states {
            for a in trace(v, 0.0, 6.0) {
                for side in 0..2 {
                    let w = wing_angles(&a, side);
                    assert!(w.shoulder.abs() <= SHOULDER_LIMIT && w.wrist.abs() <= WRIST_LIMIT);
                    for s in [w.sweep, w.wrist_sweep] {
                        assert!((SWEEP_LIMITS.0..=SWEEP_LIMITS.1).contains(&s), "{v:?}");
                    }
                    assert!((0.0..=1.0).contains(&w.fan));
                }
                let p = pose(&a, v);
                assert!(
                    p.joints
                        .values()
                        .all(|q| q.is_finite() && q.is_normalized())
                );
                assert!(p.joints.contains_key(&Joint::Primary(1, PRIMARIES - 1)));
            }
        }
    }

    #[test]
    fn the_body_rises_on_the_downstroke_and_banks_tuck_the_inner_wing() {
        let v = UnicornView {
            climb_mps: 8.0,
            ..view(30.0)
        };
        let t = trace(&v, 2.0, 3.0);
        // The bob's peak is just after the bottom of the downstroke.
        let (peak, _) = t
            .iter()
            .map(|a| {
                (
                    a.wing,
                    pose(a, &v).bob - 0.06 * (2.0 * TAU * a.gallop).sin(),
                )
            })
            .fold(
                (0.0, f32::MIN),
                |m, (w, b)| if b > m.1 { (w, b) } else { m },
            );
        assert!(
            (DOWNSTROKE..DOWNSTROKE + 0.35).contains(&peak),
            "rises after the downstroke: {peak}"
        );
        // A right turn (negative turn rate): the right wing is inside.
        let right = UnicornView {
            turn_rate: -0.8,
            ..view(40.0)
        };
        let a = run(&right, 3.0);
        assert!(a.bank > 0.3);
        let (l, r) = (wing_angles(&a, 0), wing_angles(&a, 1));
        assert!(r.wrist_sweep > l.wrist_sweep + 0.2, "{l:?} {r:?}");
        assert!(r.sweep > l.sweep);
    }

    #[test]
    fn landing_brakes_with_big_flaps_then_folds() {
        let v = UnicornView {
            landing: true,
            ..view(60.0)
        };
        let mut a = run(&view(60.0), 2.0);
        let mut deepest = 0.0f32;
        for _ in 0..(BRAKE_S * 60.0) as usize {
            a.advance(&v, 1.0 / 60.0);
            deepest = deepest.max(a.flap);
            assert!(a.spread > 0.7, "spread while braking");
        }
        assert!(deepest > 1.0, "big braking flaps: {deepest}");
        for _ in 0..120 {
            a.advance(&v, 1.0 / 60.0);
        }
        assert!(a.flap < 0.05 && a.spread < 0.05, "folded: {a:?}");
    }

    #[test]
    fn gallops_with_the_speed_and_tucks_the_legs_in_when_fast() {
        let slow = view(10.0);
        let mut a = run(&slow, 1.0);
        let g0 = a.gallop;
        a.advance(&slow, 0.1);
        let slow_rate = (a.gallop - g0).rem_euclid(1.0);
        let fast = view(40.0);
        let mut b = run(&fast, 1.0);
        let g1 = b.gallop;
        b.advance(&fast, 0.1);
        assert!((b.gallop - g1).rem_euclid(1.0) > slow_rate);
        // At 400 m/s the front legs are folded forward, still.
        let p = pose(&run(&view(400.0), 1.0), &view(400.0));
        let hip = p.joints[&Joint::Hip(0)];
        assert!((hip.angle_between(Quat::from_rotation_x(1.0))).abs() < 1e-3);
        // Banking into a left turn lowers the left wing.
        let turn = UnicornView {
            turn_rate: 0.8,
            ..view(60.0)
        };
        assert!(run(&turn, 3.0).bank < -0.1);
    }

    #[test]
    fn the_transformation_grows_the_unicorn_in() {
        assert_eq!(appear_scale(0.0), 0.0);
        assert!(appear_scale(0.4) > 1.0, "overshoot");
        assert!((appear_scale(0.6) - 1.0).abs() < 1e-6);
        assert!((appear_scale(9.0) - 1.0).abs() < 1e-6);
        let mut v = view(0.0);
        v.age_s = 0.1;
        assert!(!v.turned());
        v.age_s = 0.35;
        assert!(v.turned());
    }

    #[test]
    fn the_trail_fades_and_has_eight_bands() {
        let pts: VecDeque<(f32, Vec3)> = (0..10)
            .map(|i| (i as f32 * 0.2, Vec3::new(0.0, 10.0, i as f32 * 5.0)))
            .collect();
        let (pos, col, idx) = trail_mesh(&pts, 1.8);
        assert_eq!(pos.len(), 10 * 9);
        assert_eq!(idx.len(), 9 * 8 * 6);
        // The oldest point (2 s old at 1.8 + …): fully faded; the newest not.
        assert!(col[0][3] < 0.05);
        assert!(col[pos.len() - 1][3] > 0.25);
        assert!(trail_mesh(&VecDeque::new(), 0.0).0.is_empty());
    }

    #[test]
    fn the_ring_lies_flat() {
        let r = ring_shape(2.0, 0.1);
        assert!(r.positions.iter().all(|p| p.y.abs() <= 0.1 + 1e-6));
        assert!(
            r.positions
                .iter()
                .all(|p| (p.x.hypot(p.z) - 2.0).abs() <= 0.1 + 1e-5)
        );
    }
}
