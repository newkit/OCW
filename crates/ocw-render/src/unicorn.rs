//! The unicorn of god mode (spec §14 Phase 13C): our own low-poly model,
//! made here in code (licensed like the code; no third-party asset, no
//! brand character). A white body, a spiral golden horn, a rainbow mane
//! and tail, large feathered wings like a pegasus, golden hooves; the
//! rider's colour on the saddle cloth and the mane's first band.
//!
//! - The **model** ([`model`]) is a list of flat-shaded parts on a small
//!   skeleton ([`skeleton`]): body, neck, head, four legs (hip and knee),
//!   two wings, eight mane bands, eight tail strands – about 3k triangles.
//! - The **animation** ([`AnimState::advance`], [`pose`]) is pure: a gallop
//!   in the air whose rate follows the speed (legs tucked in when fast, a
//!   gentle paddle and bob while hovering), wing beats with the climb and
//!   the acceleration (up to ~2 Hz, [`ocw_sim::god::wing_beat_hz`]), wings
//!   spread in flight and turns and folded while hovering and landing,
//!   the body banking in turns, mane and tail streaming in the wind.
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
    /// 0 left, 1 right.
    Wing(u8),
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
    /// Horn, hooves and the saddle cloth's trim.
    Gold,
    Eye,
    Muzzle,
    /// The rider's colour: the saddle cloth and the mane's first band.
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

/// A box of `size` around the origin.
pub fn slab(size: Vec3) -> Shape {
    let h = size / 2.0;
    let positions = (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { -h.x } else { h.x },
                if i & 2 == 0 { -h.y } else { h.y },
                if i & 4 == 0 { -h.z } else { h.z },
            )
        })
        .collect();
    let indices = vec![
        0, 2, 1, 1, 2, 3, // −z
        4, 5, 6, 5, 7, 6, // +z
        0, 1, 4, 1, 5, 4, // −y
        2, 6, 3, 3, 6, 7, // +y
        0, 4, 2, 2, 4, 6, // −x
        1, 3, 5, 3, 7, 5, // +x
    ];
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

/// The unicorn's parts.
pub fn model() -> Vec<Part> {
    let mut parts = Vec::new();
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
        ellipsoid(Vec3::new(0.42, 0.46, 0.9), 12, 8),
    );
    add(
        Body,
        Paint::Coat,
        ellipsoid(Vec3::new(0.4, 0.45, 0.45), 10, 7).transformed(at(Vec3::new(0.0, 0.05, -0.62))),
    );
    add(
        Body,
        Paint::Coat,
        ellipsoid(Vec3::new(0.42, 0.44, 0.46), 10, 7).transformed(at(Vec3::new(0.0, 0.04, 0.6))),
    );
    // The saddle cloth in the rider's colour, with a golden trim.
    add(
        Body,
        Paint::Rider,
        slab(Vec3::new(0.9, 0.05, 0.62)).transformed(at(Vec3::new(0.0, 0.45, -0.05))),
    );
    add(
        Body,
        Paint::Gold,
        slab(Vec3::new(0.94, 0.03, 0.68)).transformed(at(Vec3::new(0.0, 0.42, -0.05))),
    );
    // Neck: tilted forward (rest pose in [`pose`]).
    add(Neck, Paint::Coat, tube(0.23, 0.16, 0.9, 9));
    // Head: skull, muzzle, eyes, ears, the horn.
    add(
        Head,
        Paint::Coat,
        ellipsoid(Vec3::new(0.17, 0.2, 0.27), 10, 7).transformed(at(Vec3::new(0.0, 0.05, -0.12))),
    );
    add(
        Head,
        Paint::Muzzle,
        ellipsoid(Vec3::new(0.13, 0.14, 0.2), 9, 6).transformed(at(Vec3::new(0.0, -0.03, -0.42))),
    );
    for x in [-0.135, 0.135] {
        add(
            Head,
            Paint::Eye,
            ellipsoid(Vec3::splat(0.035), 6, 4).transformed(at(Vec3::new(x, 0.11, -0.24))),
        );
        add(
            Head,
            Paint::Coat,
            tube(0.05, 0.0, 0.17, 5).transformed(Transform {
                translation: Vec3::new(x * 0.6, 0.21, -0.02),
                rotation: Quat::from_rotation_z(-x * 2.0),
                ..default()
            }),
        );
    }
    add(
        Head,
        Paint::Gold,
        horn(0.48, 0.055, 3.0).transformed(Transform {
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
            tube(0.11, 0.075, 0.55, 7).transformed(flip()),
        );
        add(
            Knee(leg),
            Paint::Coat,
            tube(0.07, 0.06, 0.48, 7).transformed(flip()),
        );
        add(
            Knee(leg),
            Paint::Gold,
            tube(0.085, 0.075, 0.12, 7).transformed(Transform {
                translation: Vec3::new(0.0, -0.48, 0.0),
                rotation: Quat::from_rotation_x(PI),
                ..default()
            }),
        );
    }
    // Wings: a covert along the arm and twelve flight feathers fanning
    // from it (the right wing along +x, the left mirrored).
    for side in 0..2u8 {
        let sx = if side == 0 { -1.0 } else { 1.0 };
        add(
            Wing(side),
            Paint::Feather,
            ellipsoid(Vec3::new(0.78, 0.06, 0.17), 10, 6).transformed(at(Vec3::new(
                sx * 0.78,
                0.0,
                0.0,
            ))),
        );
        for k in 0..12 {
            let t = k as f32 / 11.0;
            let a = t.powf(1.5) * 1.15 * sx;
            let len = 0.5 + 0.55 * t.powf(0.8);
            let dir = Vec3::new(a.sin(), 0.0, a.cos());
            let base = Vec3::new(sx * (0.12 + 1.42 * t), -0.01 * k as f32 * 0.2, 0.06);
            add(
                Wing(side),
                Paint::Feather,
                ellipsoid(Vec3::new(0.075, 0.016, len / 2.0), 6, 4).transformed(Transform {
                    translation: base + dir * (len / 2.0),
                    rotation: Quat::from_rotation_y(a),
                    ..default()
                }),
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
            ellipsoid(Vec3::new(0.03, 0.11, 0.15), 6, 4).transformed(at(Vec3::new(0.0, 0.0, 0.11))),
        );
        add(Tail(i), Paint::Band(i), tube(0.055, 0.012, 1.1, 5));
    }
    parts
}

/// Turns a +y shape to point down (legs hang from their joints).
fn flip() -> Transform {
    Transform::from_rotation(Quat::from_rotation_x(PI))
}

/// The animation state of one unicorn (pure; advanced each frame).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnimState {
    /// Gallop cycle (0 … 1, wraps).
    pub gallop: f32,
    /// Wing beat cycle (0 … 1, wraps).
    pub wing: f32,
    /// Wing beat amplitude (smoothed; 0 = gliding).
    pub flap: f32,
    /// Wing spread (smoothed; 0 folded … 1 wide).
    pub spread: f32,
    /// Body bank and pitch (smoothed, rad).
    pub bank: f32,
    pub pitch: f32,
    /// Time (s), for the mane and the tail.
    pub time: f32,
}

fn approach(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    current + (target - current) * (1.0 - (-dt / tau).exp())
}

impl AnimState {
    /// Advances by `dt` seconds of the flight `v`.
    pub fn advance(&mut self, v: &UnicornView, dt: f32) {
        use ocw_sim::god;
        let speed = v.speed_mps;
        let beat = god::wing_beat_hz(v.climb_mps, v.accel, speed) as f32;
        self.gallop = (self.gallop + god::gallop_hz(speed) as f32 * dt).fract();
        self.wing = (self.wing + beat.max(0.4) * dt).fract();
        let flap = if v.landing {
            0.0
        } else {
            (beat / 2.0).clamp(0.0, 1.0)
        };
        self.flap = approach(self.flap, flap, dt, 0.3);
        let spread = if v.landing {
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

/// The pose: each joint's rotation, the body's bob (m) and its tilt.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub joints: BTreeMap<Joint, Quat>,
    pub bob: f32,
    /// The body's pitch (nose up, rad) and bank (right wing down, rad).
    pub pitch: f32,
    pub bank: f32,
    /// Wing flap angle (rad, tips up) and spread (0 … 1), for tests.
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
    joints.insert(
        Joint::Neck,
        Quat::from_rotation_x(-0.6 + 0.05 * (2.0 * g).sin()),
    );
    joints.insert(Joint::Head, Quat::from_rotation_x(0.22));
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
    // Wings: swept back along the body when folded, out when spread;
    // beating about the shoulder, a slight dihedral when gliding.
    let flap = a.flap * 0.6 * (TAU * a.wing).sin() + 0.1 * a.spread;
    for side in 0..2u8 {
        let sx = if side == 0 { -1.0 } else { 1.0 };
        let sweep = (1.0 - a.spread) * 1.35;
        let fold_droop = -0.2 * (1.0 - a.spread);
        joints.insert(
            Joint::Wing(side),
            Quat::from_rotation_y(-sx * sweep) * Quat::from_rotation_z(sx * (flap + fold_droop)),
        );
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
            Quat::from_rotation_x(1.9 - 0.35 * wind + 0.12 * wave)
                * Quat::from_rotation_z((k - 3.5) * 0.07 * (1.0 + wind)),
        );
    }
    Pose {
        joints,
        bob: 0.06 * (2.0 * g).sin() * (1.0 - tucked) + 0.08 * hovering * (a.time * 1.6).sin(),
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

#[derive(Component)]
struct NameTag;

/// The unicorns' entities and the shared assets.
#[derive(Resource, Default)]
struct Unicorns {
    roots: BTreeMap<RiderId, (Entity, Vec3)>,
    trails: BTreeMap<RiderId, Entity>,
    tags: BTreeMap<RiderId, Entity>,
    meshes: Vec<(Joint, Paint, Handle<Mesh>)>,
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
    roots: Query<(), With<UnicornRoot>>,
) {
    let u = &mut *u;
    if u.meshes.is_empty() {
        u.meshes = model()
            .iter()
            .map(|p| (p.joint, p.paint, meshes.add(to_mesh(&p.shape))))
            .collect();
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
        for (joint, paint, mesh) in &u.meshes {
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
                ChildOf(joints[joint]),
            ));
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
const TRAIL_BAND_M: f32 = 0.16;

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
            col.push([c.red, c.green, c.blue, 0.55 * fade * fade]);
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
        // A golden horn, the rider's colour twice (saddle cloth, first
        // mane band), the rainbow in mane and tail, two wings of feathers.
        assert!(
            parts
                .iter()
                .any(|p| p.joint == Joint::Head && p.paint == Paint::Gold)
        );
        assert_eq!(parts.iter().filter(|p| p.paint == Paint::Rider).count(), 2);
        assert_eq!(
            parts
                .iter()
                .filter(|p| matches!(p.joint, Joint::Tail(_)) && matches!(p.paint, Paint::Band(_)))
                .count(),
            8
        );
        assert!(parts.iter().filter(|p| p.joint == Joint::Wing(0)).count() >= 12);
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
        assert!(col[pos.len() - 1][3] > 0.4);
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
