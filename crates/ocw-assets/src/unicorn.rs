//! The unicorn of god mode as a model file (spec §14 Phase 15, M4; the
//! unicorn itself is Phase 13C): `creatures/unicorn-winged.glb`, our own
//! low-poly model (licensed like the code; no third-party asset, no brand
//! character) – a white body, a spiral golden horn, a rainbow mane and
//! tail, large feathered wings like a pegasus, golden hooves; the rider's
//! colour in the mane's first band.
//!
//! - **The skeleton** ([`skeleton`], [`Joint`]): body, neck, head, four
//!   legs (hip and knee), two wings (shoulder, wrist and three groups of
//!   primaries), eight mane bands, eight tail strands. In the file every
//!   joint is a node named by [`Joint::node`] (`body`, `neck`, `hip_fl`,
//!   `wing_l`, `wrist_l`, `primaries_l_0`, `mane_3`, `tail_7`, …) at its
//!   pivot; the renderer's procedural flight animation (`ocw-render`'s
//!   `unicorn::pose`) sets the rotation of these nodes – the wing beat, the
//!   gallop, the mane and tail in the wind – and the body's pose (bob,
//!   pitch, bank).
//! - **The levels of detail:** the near model on the joint nodes (about 3k
//!   triangles), the far one (≈ 600) under `lod1` on a second skeleton of
//!   the same joints, named `<joint>_lod1` ([`far_node`]); the impostor
//!   card beyond 500 m stays a drawn picture in the renderer.
//! - **Paint slots** ([`Paint::slot`]): `coat`, `horn` (horn and hooves), `eye`, `muzzle`,
//!   `feather`, `mane_rider` (drawn in the rider's colour) and the rainbow
//!   bands `rainbow_0` … `rainbow_7` ([`RAINBOW`]).
//! - **Origin** (15B, a creature): on the ground under the body; the body
//!   joint is [`STANDING_HEIGHT_M`] above it, the file's rest pose a
//!   standing unicorn with spread wings ([`rest_pose`]).
//!
//! [`parts`] is the generator that wrote the file (spec §14 Phase 15 Q6:
//! the file is the source; the generator stays the exporter until all
//! generators are removed in one step). The apps read only the file.

use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

use glam::{Quat, Vec3};

use crate::model::{Material, Model, Node, Primitive, srgb};

/// The unicorn's file in `artwork/3dmodels/` (15H name).
pub const FILE: &str = "creatures/unicorn-winged.glb";

/// The body joint's height above the file's origin (m): the unicorn
/// stands on its hooves (hip 0.2 m under the body, thigh 0.55 m, shank
/// 0.48 m, hoof 0.12 m).
pub const STANDING_HEIGHT_M: f32 = 1.35;

/// The suffix of a joint's node in the far skeleton under `lod1`.
pub const FAR_SUFFIX: &str = "_lod1";

/// The eight colours of the mane, the tail and the trail (sRGB).
pub const RAINBOW: [u32; 8] = [
    0xE53935, 0xFB8C00, 0xFDD835, 0x43A047, 0x00ACC1, 0x1E88E5, 0x5E35B1, 0xD81B60,
];

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

impl Joint {
    /// The joint's node name in the file.
    pub fn node(self) -> String {
        let side = |s: u8| if s == 0 { "l" } else { "r" };
        let leg = |l: u8| ["fl", "fr", "hl", "hr"][usize::from(l % 4)];
        match self {
            Joint::Body => "body".into(),
            Joint::Neck => "neck".into(),
            Joint::Head => "head".into(),
            Joint::Hip(l) => format!("hip_{}", leg(l)),
            Joint::Knee(l) => format!("knee_{}", leg(l)),
            Joint::Wing(s) => format!("wing_{}", side(s)),
            Joint::WingTip(s) => format!("wrist_{}", side(s)),
            Joint::Primary(s, k) => format!("primaries_{}_{k}", side(s)),
            Joint::Mane(i) => format!("mane_{i}"),
            Joint::Tail(i) => format!("tail_{i}"),
        }
    }

    /// The joint of a node name ([`Joint::node`]; the far skeleton's
    /// `<joint>_lod1` too, with `true`).
    pub fn parse(name: &str) -> Option<(Joint, bool)> {
        let (base, far) = match name.strip_suffix(FAR_SUFFIX) {
            Some(b) => (b, true),
            None => (name, false),
        };
        skeleton()
            .into_iter()
            .map(|d| d.joint)
            .find(|j| j.node() == base)
            .map(|j| (j, far))
    }
}

/// The node name of `joint` in the far skeleton (under `lod1`).
pub fn far_node(joint: Joint) -> String {
    format!("{}{FAR_SUFFIX}", joint.node())
}

impl Paint {
    /// Every paint of the model.
    pub fn all() -> Vec<Paint> {
        let mut v = vec![
            Paint::Coat,
            Paint::Gold,
            Paint::Eye,
            Paint::Muzzle,
            Paint::Rider,
            Paint::Feather,
        ];
        v.extend((0..STRANDS).map(Paint::Band));
        v
    }

    /// The paint slot (material name) of the file.
    pub fn slot(self) -> &'static str {
        const BANDS: [&str; 8] = [
            "rainbow_0",
            "rainbow_1",
            "rainbow_2",
            "rainbow_3",
            "rainbow_4",
            "rainbow_5",
            "rainbow_6",
            "rainbow_7",
        ];
        match self {
            Paint::Coat => "coat",
            Paint::Gold => "horn",
            Paint::Eye => "eye",
            Paint::Muzzle => "muzzle",
            Paint::Rider => "mane_rider",
            Paint::Band(i) => BANDS[usize::from(i) % BANDS.len()],
            Paint::Feather => "feather",
        }
    }

    /// The paint of a slot ([`Paint::slot`]).
    pub fn from_slot(name: &str) -> Option<Paint> {
        Paint::all().into_iter().find(|p| p.slot() == name)
    }
}

/// A rigid placement of a shape in its joint's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub translation: Vec3,
    pub rotation: Quat,
}

impl Default for Place {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

impl Place {
    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        self.rotation * p + self.translation
    }
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
    pub fn transformed(mut self, t: Place) -> Self {
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

fn at(t: Vec3) -> Place {
    Place {
        translation: t,
        ..Place::default()
    }
}

/// The generator: the unicorn's parts, near (`true`) or far. The far model
/// has the same parts on the same joints with fewer sides, five broader
/// feathers per wing and no eyes.
pub fn parts(near: bool) -> Vec<Part> {
    let mut parts = Vec::new();
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
            tube(0.05, 0.0, 0.17, n(5, 3)).transformed(Place {
                translation: Vec3::new(x * 0.6, 0.21, -0.02),
                rotation: Quat::from_rotation_z(-x * 2.0),
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
        horn_shape.transformed(Place {
            translation: Vec3::new(0.0, 0.2, -0.24),
            rotation: Quat::from_rotation_x(-0.85),
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
            tube(0.085, 0.075, 0.12, n(7, 4)).transformed(Place {
                translation: Vec3::new(0.0, -0.48, 0.0),
                rotation: Quat::from_rotation_x(PI),
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
                    Place {
                        translation: base + dir * (len / 2.0),
                        rotation: Quat::from_rotation_y(a),
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

/// Turns a +y shape to point down (legs hang from their joints).
fn flip() -> Place {
    Place {
        rotation: Quat::from_rotation_x(PI),
        ..Place::default()
    }
}

/// The file's rest pose (what a viewer shows: standing, the neck forward,
/// the wings spread, mane and tail hanging back). The animation sets every
/// joint's rotation each frame, so the rest pose only shapes the file in
/// a viewer or in Blender; the pivots are what the animation keeps.
pub fn rest_pose() -> BTreeMap<Joint, Quat> {
    let mut m = BTreeMap::new();
    m.insert(Joint::Neck, Quat::from_rotation_x(-0.6));
    m.insert(Joint::Head, Quat::from_rotation_x(0.22));
    for i in 0..STRANDS {
        let k = f32::from(i);
        m.insert(Joint::Mane(i), Quat::from_rotation_x(0.25));
        m.insert(
            Joint::Tail(i),
            Quat::from_rotation_x(1.9) * Quat::from_rotation_z((k - 3.5) * 0.07),
        );
    }
    m
}

/// The unicorn's file model: the root `unicorn` on the ground, the near
/// skeleton from `body` ([`STANDING_HEIGHT_M`] up) with the near parts on
/// their joints, and `lod1` with the far skeleton (`<joint>_lod1`) and the
/// far parts. Each joint holds one primitive per paint slot (its parts
/// merged, flat-shaded: three vertices per triangle with the face's
/// normal).
pub fn model() -> Model {
    let mut m = Model::default();
    let root = m.add(Node::new("unicorn", None, Vec3::ZERO));
    let rest = rest_pose();
    for near in [true, false] {
        let top = if near {
            root
        } else {
            m.add(Node::new(
                crate::pose::bike_nodes::LOD1,
                Some(root),
                Vec3::ZERO,
            ))
        };
        let mut idx: BTreeMap<Joint, usize> = BTreeMap::new();
        for d in skeleton() {
            let parent = d.parent.map_or(top, |p| idx[&p]);
            let pivot = if d.parent.is_none() {
                Vec3::Y * STANDING_HEIGHT_M
            } else {
                d.pivot
            };
            let name = if near {
                d.joint.node()
            } else {
                far_node(d.joint)
            };
            let mut node = Node::new(&name, Some(parent), pivot);
            node.rotation = rest.get(&d.joint).copied().unwrap_or(Quat::IDENTITY);
            idx.insert(d.joint, m.add(node));
        }
        type Group = ((Joint, Paint), Vec<[f32; 3]>, Vec<[f32; 3]>);
        let mut groups: Vec<Group> = Vec::new();
        for p in parts(near) {
            let (pos, nor) = p.shape.flat();
            let key = (p.joint, p.paint);
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => {
                    g.1.extend(pos);
                    g.2.extend(nor);
                }
                None => groups.push((key, pos, nor)),
            }
        }
        for ((joint, paint), positions, normals) in groups {
            let material = m.material_or_add(slot_material(paint));
            let n = positions.len() as u32;
            m.nodes[idx[&joint]].primitives.push(Primitive {
                material,
                positions,
                normals,
                indices: (0..n).collect(),
                ..Default::default()
            });
        }
    }
    depth_first(m)
}

/// The model with its nodes in the order a glTF reader walks them
/// (depth-first, children in their order), so the file reads back node
/// for node.
fn depth_first(m: Model) -> Model {
    let mut order = Vec::with_capacity(m.nodes.len());
    let mut stack: Vec<usize> = (0..m.nodes.len())
        .filter(|i| m.nodes[*i].parent.is_none())
        .rev()
        .collect();
    while let Some(i) = stack.pop() {
        order.push(i);
        let children: Vec<usize> = (0..m.nodes.len())
            .filter(|c| m.nodes[*c].parent == Some(i))
            .collect();
        stack.extend(children.into_iter().rev());
    }
    let mut new_index = vec![0; m.nodes.len()];
    for (k, i) in order.iter().enumerate() {
        new_index[*i] = k;
    }
    let mut nodes = Vec::with_capacity(m.nodes.len());
    for i in order {
        let mut n = m.nodes[i].clone();
        n.parent = n.parent.map(|p| new_index[p]);
        nodes.push(n);
    }
    Model { nodes, ..m }
}

/// The file's material of a paint (its slot's default colour; the
/// renderer draws the unicorn's own materials by slot).
fn slot_material(paint: Paint) -> Material {
    let m = crate::slots::material(paint.slot());
    debug_assert_eq!(
        m.color,
        srgb(crate::slots::get(paint.slot()).expect("slot").color)
    );
    m
}

/// The primitives of a unicorn file on their joints: (joint, far, paint,
/// primitive in the joint's frame).
pub type FileParts = Vec<(Joint, bool, Paint, Primitive)>;

/// The skeleton and the parts of a unicorn file (the renderer's view of
/// it): the near joints' parents and pivots (the body's pivot is the
/// file's standing height; the renderer places the body itself), and every
/// primitive on a joint, near or far. Errors name what is missing: every
/// joint of [`skeleton`] must be in the file, near and far, with the same
/// parent and pivot in both, and every material must be a unicorn slot.
pub fn read(m: &Model) -> Result<(Vec<JointDef>, FileParts), String> {
    let mut defs: Vec<JointDef> = Vec::new();
    let mut far_defs: BTreeMap<Joint, (Option<Joint>, Vec3)> = BTreeMap::new();
    let mut prims: FileParts = Vec::new();
    for n in &m.nodes {
        let Some((joint, far)) = Joint::parse(&n.name) else {
            continue;
        };
        let parent = n
            .parent
            .and_then(|p| Joint::parse(&m.nodes[p].name))
            .map(|(j, _)| j);
        if far {
            far_defs.insert(joint, (parent, n.translation));
        } else {
            if defs.iter().any(|d| d.joint == joint) {
                return Err(format!("joint node `{}` twice", n.name));
            }
            defs.push(JointDef {
                joint,
                parent,
                pivot: n.translation,
            });
        }
        for p in &n.primitives {
            let name = &m.materials[p.material].name;
            let paint = Paint::from_slot(name)
                .ok_or_else(|| format!("`{}`: material `{name}` is no unicorn slot", n.name))?;
            prims.push((joint, far, paint, p.clone()));
        }
    }
    let skel = skeleton();
    for d in &skel {
        let Some(own) = defs.iter().find(|e| e.joint == d.joint) else {
            return Err(format!("no joint node `{}`", d.joint.node()));
        };
        if own.parent != d.parent {
            return Err(format!("`{}` hangs on the wrong parent", d.joint.node()));
        }
        match far_defs.get(&d.joint) {
            None => return Err(format!("no far joint node `{}`", far_node(d.joint))),
            Some((parent, pivot)) => {
                if *parent != own.parent || pivot.distance(own.pivot) > 1e-6 {
                    return Err(format!(
                        "`{}` is not where `{}` is",
                        far_node(d.joint),
                        d.joint.node()
                    ));
                }
            }
        }
    }
    // Parents before children, as in the skeleton.
    defs.sort_by_key(|d| {
        skel.iter()
            .position(|e| e.joint == d.joint)
            .unwrap_or(usize::MAX)
    });
    Ok((defs, prims))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tris(near: bool) -> usize {
        parts(near).iter().map(|p| p.shape.triangles()).sum()
    }

    #[test]
    fn the_model_is_low_poly_and_complete() {
        let near = parts(true);
        assert!((2_500..=3_500).contains(&tris(true)), "near {}", tris(true));
        assert!((450..=800).contains(&tris(false)), "far {}", tris(false));
        let skel = skeleton();
        for p in &near {
            assert!(skel.iter().any(|d| d.joint == p.joint), "{:?}", p.joint);
        }
        for (i, d) in skel.iter().enumerate() {
            if let Some(parent) = d.parent {
                assert!(skel[..i].iter().any(|e| e.joint == parent));
            }
        }
        assert!(
            near.iter()
                .any(|p| p.joint == Joint::Head && p.paint == Paint::Gold)
        );
        for lod in [true, false] {
            let ps = parts(lod);
            let rider: Vec<_> = ps.iter().filter(|p| p.paint == Paint::Rider).collect();
            assert_eq!(rider.len(), 1, "{lod}");
            assert_eq!(rider[0].joint, Joint::Mane(0), "{lod}");
            assert!(
                ps.iter()
                    .all(|p| p.joint != Joint::Body || p.paint == Paint::Coat)
            );
            assert!(ps.iter().any(|p| p.joint == Joint::WingTip(1)));
            for k in 0..PRIMARIES {
                assert!(ps.iter().any(|p| p.joint == Joint::Primary(1, k)));
            }
        }
        assert_eq!(
            near.iter()
                .filter(|p| matches!(p.joint, Joint::Tail(_)) && matches!(p.paint, Paint::Band(_)))
                .count(),
            8
        );
        let left = |p: &&Part| {
            matches!(
                p.joint,
                Joint::Wing(0) | Joint::WingTip(0) | Joint::Primary(0, _)
            )
        };
        assert!(near.iter().filter(left).count() >= 14);
        // The far model keeps every joint moving.
        let joints = |lod| {
            let mut j: Vec<Joint> = parts(lod).iter().map(|p| p.joint).collect();
            j.sort();
            j.dedup();
            j
        };
        assert_eq!(joints(true), joints(false));
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
    fn joint_names_and_slots_round_trip() {
        let mut names: Vec<String> = skeleton().iter().map(|d| d.joint.node()).collect();
        for d in skeleton() {
            assert_eq!(Joint::parse(&d.joint.node()), Some((d.joint, false)));
            assert_eq!(Joint::parse(&far_node(d.joint)), Some((d.joint, true)));
            names.push(far_node(d.joint));
        }
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n, "unique names");
        assert_eq!(Joint::parse("unicorn"), None);
        for p in Paint::all() {
            assert_eq!(Paint::from_slot(p.slot()), Some(p));
            assert!(crate::slots::get(p.slot()).is_some(), "{}", p.slot());
        }
        // The slots carry the rainbow.
        for (i, c) in RAINBOW.iter().enumerate() {
            let s = crate::slots::get(Paint::Band(i as u8).slot()).unwrap();
            assert_eq!(s.color, *c);
        }
    }

    /// The file's layout: every joint near and far at its pivot, the
    /// unicorn standing on the origin, the parts on their joints.
    #[test]
    fn the_file_model_has_the_skeleton_the_levels_and_stands_on_its_origin() {
        let m = model();
        let (defs, prims) = read(&m).unwrap();
        for d in skeleton() {
            let own = defs.iter().find(|e| e.joint == d.joint).unwrap();
            let want = if d.parent.is_none() {
                Vec3::Y * STANDING_HEIGHT_M
            } else {
                d.pivot
            };
            assert_eq!(own.pivot, want, "{:?}", d.joint);
        }
        let lod = m.node(crate::pose::bike_nodes::LOD1).unwrap();
        assert_eq!(m.triangles_below(Some(lod)), tris(false));
        assert_eq!(m.triangles() - m.triangles_below(Some(lod)), tris(true));
        assert!(prims.iter().any(|p| p.1) && prims.iter().any(|p| !p.1));
        let (lo, hi) = m.near_bounds().unwrap();
        assert!(lo.y.abs() < 1e-3, "hooves on the ground: {lo}");
        assert!(hi.y > 2.0 && hi.y < 3.5, "{hi}");
        // One primitive per joint and paint.
        let mut keys: Vec<(Joint, bool, Paint)> = prims.iter().map(|p| (p.0, p.1, p.2)).collect();
        let n = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), n);
        // A broken file is named.
        let mut broken = m.clone();
        let i = broken.node("wrist_r_lod1").unwrap();
        broken.nodes[i].name = "wrist_x".into();
        assert!(read(&broken).unwrap_err().contains("wrist_r_lod1"));
    }

    /// The shipped file holds the generator's meshes (merged per joint and
    /// paint), near and far.
    #[test]
    fn the_shipped_file_is_the_generators_unicorn() {
        let file = crate::shipped::model(crate::kinds::id_of(FILE));
        let (_, got) = read(&file).unwrap();
        let (_, want) = read(&model()).unwrap();
        assert_eq!(got.len(), want.len());
        for (a, b) in got.iter().zip(&want) {
            assert_eq!((a.0, a.1, a.2), (b.0, b.1, b.2));
            assert_eq!(a.3.indices, b.3.indices);
            for (p, q) in a.3.positions.iter().zip(&b.3.positions) {
                assert!(Vec3::from(*p).distance(Vec3::from(*q)) < 1e-4);
            }
            for (p, q) in a.3.normals.iter().zip(&b.3.normals) {
                assert!(Vec3::from(*p).distance(Vec3::from(*q)) < 1e-4);
            }
        }
    }

    /// The meshes are the ones the renderer generated before the file
    /// (v5.09, `ocw-render::unicorn::model_lod`): per joint, paint and
    /// level the same triangles, bounds, centre and area, and the same
    /// pivots.
    #[test]
    fn the_parts_match_the_old_generated_unicorn() {
        let reference = include_str!("../tests/data/unicorn-generated-v5.09.txt");
        let m = crate::shipped::model(crate::kinds::id_of(FILE));
        let (defs, prims) = read(&m).unwrap();
        let mut rows = 0;
        for line in reference.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let num = |k: usize| -> f32 { w[k].parse().unwrap() };
            match w[0] {
                "joint" => {
                    let d = skeleton()
                        .into_iter()
                        .find(|d| format!("{:?}", d.joint).replace(" ", "") == w[1])
                        .unwrap();
                    let own = defs.iter().find(|e| e.joint == d.joint).unwrap();
                    let n = w.len();
                    let pivot = Vec3::new(num(n - 3), num(n - 2), num(n - 1));
                    if d.parent.is_some() {
                        assert!(own.pivot.distance(pivot) < 1e-4, "{line}");
                    }
                }
                "part" => {
                    rows += 1;
                    let near = w[1] == "Near";
                    let p = prims
                        .iter()
                        .find(|p| {
                            p.1 != near
                                && format!("{:?}", p.0).replace(" ", "") == w[2]
                                && format!("{:?}", p.2) == w[3]
                        })
                        .unwrap_or_else(|| panic!("no part for {line}"));
                    let v: Vec<Vec3> = p.3.positions.iter().map(|q| Vec3::from(*q)).collect();
                    assert_eq!(v.len() / 3, w[4].parse::<usize>().unwrap(), "{line}");
                    let lo = v.iter().fold(Vec3::splat(f32::MAX), |a, b| a.min(*b));
                    let hi = v.iter().fold(Vec3::splat(f32::MIN), |a, b| a.max(*b));
                    let c = v.iter().sum::<Vec3>() / v.len() as f32;
                    let area: f32 = v
                        .chunks(3)
                        .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() * 0.5)
                        .sum();
                    let want = [
                        Vec3::new(num(5), num(6), num(7)),
                        Vec3::new(num(8), num(9), num(10)),
                        Vec3::new(num(11), num(12), num(13)),
                    ];
                    for (got, want) in [lo, hi, c].iter().zip(want) {
                        assert!(got.distance(want) < 1e-4, "{line}: {got}");
                    }
                    assert!((area - num(14)).abs() < 5e-4 * num(14).max(1.0), "{line}");
                }
                _ => panic!("{line}"),
            }
        }
        assert_eq!(
            rows,
            prims.len(),
            "every part of the old model, and no other"
        );
    }
}
