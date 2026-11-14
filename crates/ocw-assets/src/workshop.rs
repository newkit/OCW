//! The workshop room (spec §14 Phase 4, 4A *Rendering*, *Layout*): a
//! generated bike workshop, own work (MIT OR Apache-2.0), no brands and no
//! text. Concrete floor, light walls and a ceiling with panel lamps, the
//! **repair stand** in the foreground centre, the **back wall** with one
//! hook per owned bike (plus an empty "+" hook at the end), the
//! **workbench** with a pegboard of tools and a parts shelf on the left,
//! the **changing corner** (mirror, bench, locker) and a window on the
//! right.
//!
//! Coordinates are the app's: metres, Y up, the floor at `y = 0`, the back
//! wall at [`BACK_WALL_Z`], the camera on the +Z side (the room has no
//! front wall). [`layout`] places everything, [`room`] builds the meshes
//! for that layout; the bikes are not part of the room (the renderer puts
//! the bike models at [`Layout::stand_bike`] and [`Hook::bike_pose`]).
//!
//! Materials are this module's own ([`MATERIALS`]), not paint slots: the
//! room is never recoloured. `lamp` and `window_glass` are meant to be
//! drawn emissive (light sources seen in the room).

use std::collections::BTreeMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::{Mat3, Quat, Vec2, Vec3};

use crate::mesh::Shape;
use crate::model::{Material, Model, Node, Primitive, srgb};

/// Inner face of the back wall (the hook wall).
pub const BACK_WALL_Z: f32 = -3.2;
/// Where the side walls and the ceiling end towards the camera.
pub const FRONT_Z: f32 = 4.5;
/// Ceiling height.
pub const CEILING_Y: f32 = 3.0;
/// Height of the hooks (where they hold the front rim's top).
pub const HOOK_Y: f32 = 2.55;
/// Hook spacing along the back wall.
pub const HOOK_SPACING: f32 = 1.25;
/// Angle between a hung bike's plane and the back wall (rad): turned out
/// from the wall, so its side shows (60°).
pub const HANG_ANGLE: f32 = PI / 3.0;
/// The front hub of a hung bike, out from the back wall.
pub const HANG_OUT: f32 = 0.56;
/// A bike on the stand: its wheels this far above the floor.
pub const STAND_LIFT: f32 = 0.25;
/// Depth (z) of the bike on the stand.
pub const STAND_Z: f32 = 0.6;
/// Smallest half width of the room (x).
const MIN_HALF_WIDTH: f32 = 4.5;
/// Nominal wheel radius / wheelbase for [`Hook::bike`].
const NOMINAL_RADIUS: f32 = 0.35;
const NOMINAL_WHEELBASE: f32 = 1.0;
/// The hook holds the rim this far inside the tyre's outer radius.
const RIM_INSET: f32 = 0.03;

/// A material of the room: name, sRGB colour, metallic, roughness.
pub const MATERIALS: [(&str, u32, f32, f32); 26] = [
    ("concrete", 0x8E8B85, 0.0, 0.92),
    ("concrete_dark", 0x77746F, 0.0, 0.95),
    ("wall", 0xEAE3D6, 0.0, 0.88),
    ("wall_accent", 0x5E7A6E, 0.0, 0.85),
    ("skirting", 0x6B5645, 0.0, 0.7),
    ("ceiling", 0xF1EEE8, 0.0, 0.9),
    ("lamp", 0xFFF4DC, 0.0, 0.4),
    ("wood", 0xB57E48, 0.0, 0.62),
    ("wood_dark", 0x5C4030, 0.0, 0.7),
    ("pegboard", 0xC9A27A, 0.0, 0.8),
    ("peg_hole", 0x3A2E24, 0.0, 0.9),
    ("steel", 0xA2A7AC, 0.85, 0.32),
    ("steel_dark", 0x2C2E33, 0.4, 0.45),
    ("rubber", 0x262626, 0.0, 0.9),
    ("tool_red", 0xC0392B, 0.0, 0.45),
    ("tool_blue", 0x2E6DB4, 0.0, 0.45),
    ("tool_yellow", 0xE3B11F, 0.0, 0.5),
    ("cardboard", 0xB58B5A, 0.0, 0.85),
    ("bin_blue", 0x3D6F95, 0.0, 0.55),
    ("bin_grey", 0x8C9196, 0.0, 0.6),
    ("locker", 0x5B7F95, 0.5, 0.45),
    ("mirror", 0xDDE3E8, 1.0, 0.04),
    ("window_glass", 0xCFE4F2, 0.0, 0.08),
    ("window_frame", 0xF4F4F0, 0.0, 0.5),
    ("card", 0xF4F1EA, 0.0, 0.7),
    ("stand", 0x3A3F47, 0.6, 0.38),
];

fn material(name: &str) -> Material {
    let (n, c, m, r) = MATERIALS
        .iter()
        .find(|(n, ..)| *n == name)
        .copied()
        .unwrap_or_else(|| panic!("unknown workshop material {name}"));
    Material {
        name: n.into(),
        color: srgb(c),
        metallic: m,
        roughness: r,
    }
}

/// One wall hook: the hung bike, the hook and its name card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hook {
    /// Pose (translation of the model origin, rotation) of a bike with the
    /// nominal wheel size; [`Hook::bike_pose`] for a given bike.
    pub bike: (Vec3, Quat),
    /// Where the hook holds the front rim (its top).
    pub hook: Vec3,
    /// The name card plate on the wall under the bike (its centre).
    pub card: Vec3,
    /// The rotation of every hung bike: front wheel up, its plane turned
    /// [`HANG_ANGLE`] out from the wall, the drive side towards the camera.
    pub rotation: Quat,
}

impl Hook {
    /// The pose of a bike with this outer wheel radius and wheelbase
    /// ([`crate::pose::BikeRig::wheel_radius`], `wheelbase`): its front
    /// rim hangs on the hook.
    pub fn bike_pose(&self, wheel_radius: f32, wheelbase: f32) -> (Vec3, Quat) {
        let front_hub_world = self.hook - Vec3::Y * (wheel_radius - RIM_INSET);
        let front_hub_local = Vec3::new(0.0, wheel_radius, -wheelbase * 0.5);
        (
            front_hub_world - self.rotation * front_hub_local,
            self.rotation,
        )
    }
}

/// A camera view: eye and target (the renderer's camera looks from `eye`
/// at `target`, Y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub eye: Vec3,
    pub target: Vec3,
}

/// Where everything in the workshop is.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Pose of the bike on the stand (model origin, rotation): lifted by
    /// [`STAND_LIFT`], turned by −90° about Y (drive side, the bike's +X,
    /// towards +Z; front to +X).
    pub stand_bike: (Vec3, Quat),
    /// Foot of the stand's post (the tripod's centre, on the floor).
    pub stand_base: Vec3,
    /// The clamp on the seat post of a road bike on the stand.
    pub stand_clamp: Vec3,
    /// The wall hooks from left to right: one per owned bike, the last one
    /// the empty "+" hook.
    pub hooks: Vec<Hook>,
    /// The workbench's worktop, centre (left of the stand).
    pub bench: Vec3,
    /// The mirror's centre (changing corner, right wall; it faces −X).
    pub mirror: Vec3,
    /// The shadow-casting spot over the stand.
    pub spot: Vec3,
    /// Fill lights (point lights under the ceiling).
    pub fills: Vec<Vec3>,
    /// The window: centre and size (width along the wall, height); on the
    /// right wall, facing −X.
    pub window: (Vec3, Vec2),
    /// The room's inside: min and max corner.
    pub bounds: (Vec3, Vec3),
}

/// The rotation of a hung bike ([`Hook::rotation`]).
pub fn hang_rotation() -> Quat {
    let (s, c) = HANG_ANGLE.sin_cos();
    // Bike axes → world: front (−Z) up; up (Y) out from the wall and to the
    // left; right (X) mostly towards the camera.
    let x = Vec3::new(s, 0.0, c);
    let y = Vec3::new(-c, 0.0, s);
    Quat::from_mat3(&Mat3::from_cols(x, y, Vec3::NEG_Y))
}

/// The layout for `bikes` owned bikes (at least one hook: the "+" hook;
/// up to 24 bikes, the room grows along the back wall).
pub fn layout(bikes: usize) -> Layout {
    let n = bikes.min(24) + 1;
    let half = MIN_HALF_WIDTH.max(n as f32 * HOOK_SPACING * 0.5 + 1.3);
    let rotation = hang_rotation();
    let hooks = (0..n)
        .map(|i| {
            let x = (i as f32 - (n as f32 - 1.0) * 0.5) * HOOK_SPACING;
            let mut h = Hook {
                bike: (Vec3::ZERO, rotation),
                hook: Vec3::new(x, HOOK_Y, BACK_WALL_Z + HANG_OUT),
                card: Vec3::new(x, 0.38, BACK_WALL_Z + 0.012),
                rotation,
            };
            h.bike = h.bike_pose(NOMINAL_RADIUS, NOMINAL_WHEELBASE);
            h
        })
        .collect();
    let stand_rot = Quat::from_rotation_y(-FRAC_PI_2);
    let stand_at = Vec3::new(0.0, STAND_LIFT, STAND_Z);
    let saddle = crate::shipped::bike_rig(ocw_core::ProfileId::Road).saddle;
    let clamp_local = saddle + Vec3::new(0.0, -0.16, -0.07);
    let stand_clamp = stand_at + stand_rot * clamp_local;
    Layout {
        stand_bike: (stand_at, stand_rot),
        stand_base: Vec3::new(stand_clamp.x, 0.0, STAND_Z - 0.42),
        stand_clamp,
        hooks,
        bench: Vec3::new(-half + 0.4, 0.92, -1.0),
        mirror: Vec3::new(half - 0.04, 1.25, 0.9),
        spot: Vec3::new(0.0, CEILING_Y - 0.12, STAND_Z + 0.35),
        fills: vec![
            Vec3::new(-half * 0.55, CEILING_Y - 0.3, 0.2),
            Vec3::new(half * 0.55, CEILING_Y - 0.3, 0.2),
            Vec3::new(0.0, CEILING_Y - 0.3, BACK_WALL_Z + 1.0),
            Vec3::new(0.0, CEILING_Y - 0.3, 2.8),
        ],
        window: (Vec3::new(half - 0.02, 1.7, -0.5), Vec2::new(0.8, 1.0)),
        bounds: (
            Vec3::new(-half, 0.0, BACK_WALL_Z),
            Vec3::new(half, CEILING_Y, FRONT_Z),
        ),
    }
}

impl Layout {
    fn half_width(&self) -> f32 {
        self.bounds.1.x
    }

    /// Suggested *Overview* view: the stand and the wall.
    pub fn overview_view(&self) -> View {
        View {
            eye: Vec3::new(0.0, 1.75, 4.1),
            target: Vec3::new(0.0, 1.25, -1.6),
        }
    }

    /// Suggested *Wall* view of hook `i`: square on to the wall.
    pub fn wall_view(&self, i: usize) -> View {
        let h = self.hooks[i.min(self.hooks.len() - 1)];
        View {
            eye: Vec3::new(h.hook.x, 1.65, BACK_WALL_Z + 3.3),
            target: Vec3::new(h.hook.x, 1.55, BACK_WALL_Z),
        }
    }

    /// Suggested *Stand* view (orbit centre and a starting eye 1.9 m away,
    /// on the drive side).
    pub fn stand_view(&self) -> View {
        let target = self.stand_bike.0 + Vec3::Y * 0.55;
        View {
            eye: target + Vec3::new(0.25, 0.35, 1.85),
            target,
        }
    }

    /// Suggested *Mirror* view: the changing corner from over the stand.
    pub fn mirror_view(&self) -> View {
        View {
            eye: Vec3::new(self.half_width() - 2.6, 1.6, 2.4),
            target: self.mirror - Vec3::new(0.0, 0.15, 0.0),
        }
    }

    /// Suggested *Bench* view.
    pub fn bench_view(&self) -> View {
        View {
            eye: Vec3::new(-self.half_width() + 2.6, 1.7, 1.4),
            target: self.bench + Vec3::new(0.0, 0.3, -0.2),
        }
    }
}

/// Shapes by material, for one node.
#[derive(Default)]
struct Mats(BTreeMap<&'static str, Shape>);

impl Mats {
    fn s(&mut self, m: &'static str) -> &mut Shape {
        debug_assert!(MATERIALS.iter().any(|(n, ..)| *n == m), "material {m}");
        self.0.entry(m).or_default()
    }

    fn box_(&mut self, m: &'static str, centre: Vec3, half: Vec3) {
        self.s(m).cuboid(centre, half, Quat::IDENTITY);
    }

    fn attach(self, model: &mut Model, name: &str, root: usize) {
        let node = model.add(Node::new(name, Some(root), Vec3::ZERO));
        for (m, shape) in self.0 {
            if shape.indices.is_empty() {
                continue;
            }
            let material = model.material_or_add(material(m));
            model.nodes[node].primitives.push(Primitive {
                material,
                positions: shape.positions.iter().map(|p| p.to_array()).collect(),
                normals: shape.normals.iter().map(|n| n.to_array()).collect(),
                indices: shape.indices,
            });
        }
    }
}

/// The room's node names.
pub const NODES: [&str; 8] = [
    "floor",
    "walls",
    "stand",
    "hooks",
    "bench",
    "pegboard",
    "mirror_corner",
    "window",
];

/// Builds the room for `layout`.
pub fn room(layout: &Layout) -> Model {
    let mut m = Model {
        copyright: crate::COPYRIGHT.into(),
        generator: crate::GENERATOR.into(),
        ..Default::default()
    };
    let root = m.add(Node::new("workshop", None, Vec3::ZERO));
    let (lo, hi) = layout.bounds;
    floor(layout).attach(&mut m, "floor", root);
    walls(lo, hi).attach(&mut m, "walls", root);
    stand(layout).attach(&mut m, "stand", root);
    hooks(layout).attach(&mut m, "hooks", root);
    bench(layout).attach(&mut m, "bench", root);
    pegboard(layout).attach(&mut m, "pegboard", root);
    mirror_corner(layout).attach(&mut m, "mirror_corner", root);
    window(layout).attach(&mut m, "window", root);
    m
}

fn floor(l: &Layout) -> Mats {
    let mut p = Mats::default();
    let (lo, hi) = l.bounds;
    // Slab, with expansion joints in a 1.5 m grid (slightly darker strips).
    p.box_(
        "concrete",
        Vec3::new(0.0, -0.05, (lo.z + hi.z) * 0.5),
        Vec3::new(hi.x, 0.05, (hi.z - lo.z) * 0.5),
    );
    let mut x = -1.5 * (hi.x / 1.5).floor();
    while x <= hi.x {
        p.box_(
            "concrete_dark",
            Vec3::new(x, 0.0005, (lo.z + hi.z) * 0.5),
            Vec3::new(0.006, 0.0006, (hi.z - lo.z) * 0.5),
        );
        x += 1.5;
    }
    let mut z = lo.z + 1.5;
    while z < hi.z {
        p.box_(
            "concrete_dark",
            Vec3::new(0.0, 0.0005, z),
            Vec3::new(hi.x, 0.0006, 0.006),
        );
        z += 1.5;
    }
    // Rubber work mat under the stand.
    p.box_(
        "rubber",
        Vec3::new(0.0, 0.004, STAND_Z),
        Vec3::new(1.1, 0.004, 0.85),
    );
    // Floor pump beside the stand: base, barrel, T-handle, hose.
    let pump = Vec3::new(1.15, 0.0, STAND_Z - 0.35);
    p.box_(
        "steel_dark",
        pump + Vec3::new(0.0, 0.012, 0.0),
        Vec3::new(0.12, 0.012, 0.045),
    );
    p.s("tool_red").tube(
        pump + Vec3::Y * 0.02,
        pump + Vec3::Y * 0.6,
        0.024,
        0.024,
        12,
        true,
    );
    p.s("steel").tube(
        pump + Vec3::Y * 0.6,
        pump + Vec3::Y * 0.68,
        0.006,
        0.006,
        6,
        false,
    );
    p.s("rubber").tube(
        pump + Vec3::new(-0.11, 0.69, 0.0),
        pump + Vec3::new(0.11, 0.69, 0.0),
        0.014,
        0.014,
        8,
        true,
    );
    p.s("steel_dark").ellipsoid(
        pump + Vec3::new(0.0, 0.45, 0.026),
        Vec3::splat(0.03),
        Quat::IDENTITY,
        6,
        10,
    );
    p.s("rubber").sweep(
        &[
            pump + Vec3::new(0.02, 0.05, 0.02),
            pump + Vec3::new(0.12, 0.08, 0.1),
            pump + Vec3::new(0.14, 0.35, 0.08),
            pump + Vec3::new(0.05, 0.5, 0.04),
        ],
        &[0.005; 4],
        6,
        true,
    );
    p
}

fn walls(lo: Vec3, hi: Vec3) -> Mats {
    let mut p = Mats::default();
    let t = 0.1;
    let depth = hi.z - lo.z;
    let mid_z = (lo.z + hi.z) * 0.5;
    // Back wall: the lower half in the accent colour (a painted dado).
    p.box_(
        "wall",
        Vec3::new(0.0, hi.y * 0.5, lo.z - t),
        Vec3::new(hi.x + t, hi.y * 0.5 + t, t),
    );
    p.box_(
        "wall_accent",
        Vec3::new(0.0, 0.6, lo.z + 0.002),
        Vec3::new(hi.x, 0.6, 0.002),
    );
    // Side walls.
    for side in [-1.0f32, 1.0] {
        p.box_(
            "wall",
            Vec3::new(side * (hi.x + t), hi.y * 0.5, mid_z),
            Vec3::new(t, hi.y * 0.5 + t, depth * 0.5),
        );
        p.box_(
            "skirting",
            Vec3::new(side * (hi.x - 0.008), 0.05, mid_z),
            Vec3::new(0.008, 0.05, depth * 0.5),
        );
    }
    p.box_(
        "skirting",
        Vec3::new(0.0, 0.05, lo.z + 0.008),
        Vec3::new(hi.x, 0.05, 0.008),
    );
    // Ceiling with two rows of panel lamps.
    p.box_(
        "ceiling",
        Vec3::new(0.0, hi.y + t, mid_z),
        Vec3::new(hi.x + t, t, depth * 0.5),
    );
    for x in [-hi.x * 0.45, hi.x * 0.45, 0.0] {
        for z in [lo.z + 1.0, 0.6, 2.8] {
            if x == 0.0 && z != 0.6 {
                continue;
            }
            p.box_(
                "steel_dark",
                Vec3::new(x, hi.y - 0.02, z),
                Vec3::new(0.62, 0.02, 0.12),
            );
            p.box_(
                "lamp",
                Vec3::new(x, hi.y - 0.041, z),
                Vec3::new(0.58, 0.002, 0.09),
            );
        }
    }
    p
}

fn stand(l: &Layout) -> Mats {
    let mut p = Mats::default();
    let base = l.stand_base;
    let hub = base + Vec3::Y * 0.28;
    // Tripod: three legs with rubber feet.
    for k in 0..3 {
        let a = k as f32 / 3.0 * TAU + PI / 2.0;
        let foot = base + Vec3::new(a.cos() * 0.48, 0.02, a.sin() * 0.48);
        p.s("stand").tube(hub, foot, 0.016, 0.014, 8, true);
        p.s("rubber").cuboid(
            foot - Vec3::Y * 0.01,
            Vec3::new(0.03, 0.012, 0.03),
            Quat::IDENTITY,
        );
    }
    // Post (lower tube, collar, upper tube).
    let top = Vec3::new(base.x, l.stand_clamp.y, base.z);
    p.s("stand").tube(
        base + Vec3::Y * 0.05,
        hub + Vec3::Y * 0.3,
        0.025,
        0.025,
        12,
        true,
    );
    p.s("steel_dark").tube(
        hub + Vec3::Y * 0.28,
        hub + Vec3::Y * 0.34,
        0.032,
        0.032,
        12,
        true,
    );
    p.s("steel")
        .tube(hub + Vec3::Y * 0.3, top, 0.019, 0.019, 12, true);
    // Clamp arm forward to the seat post, the jaws and the knob.
    let clamp = l.stand_clamp;
    p.s("stand").tube(
        top,
        Vec3::new(clamp.x, top.y, clamp.z - 0.04),
        0.017,
        0.017,
        10,
        true,
    );
    p.s("steel_dark").cuboid(
        Vec3::new(clamp.x, clamp.y, clamp.z - 0.035),
        Vec3::new(0.035, 0.05, 0.02),
        Quat::IDENTITY,
    );
    for side in [-1.0f32, 1.0] {
        p.s("rubber").cuboid(
            clamp + Vec3::new(side * 0.03, 0.0, 0.0),
            Vec3::new(0.008, 0.04, 0.03),
            Quat::IDENTITY,
        );
    }
    p.s("tool_red").tube(
        Vec3::new(clamp.x, top.y, clamp.z - 0.06),
        Vec3::new(clamp.x, top.y - 0.12, clamp.z - 0.12),
        0.008,
        0.008,
        6,
        false,
    );
    p.s("tool_red").ellipsoid(
        Vec3::new(clamp.x, top.y - 0.13, clamp.z - 0.125),
        Vec3::splat(0.02),
        Quat::IDENTITY,
        5,
        8,
    );
    // A small tool tray on the post.
    p.s("stand").cuboid(
        hub + Vec3::new(0.0, 0.45, -0.12),
        Vec3::new(0.14, 0.008, 0.08),
        Quat::IDENTITY,
    );
    for (dx, mat) in [
        (-0.07, "tool_blue"),
        (0.0, "tool_yellow"),
        (0.07, "tool_red"),
    ] {
        p.s(mat).cuboid(
            hub + Vec3::new(dx, 0.465, -0.12),
            Vec3::new(0.012, 0.006, 0.05),
            Quat::IDENTITY,
        );
    }
    p
}

fn hooks(l: &Layout) -> Mats {
    let mut p = Mats::default();
    let plus = l.hooks.len() - 1;
    // A wooden rail along the wall that carries the hooks.
    let (first, last) = (l.hooks[0].hook.x, l.hooks[plus].hook.x);
    p.box_(
        "wood",
        Vec3::new((first + last) * 0.5, HOOK_Y + 0.04, BACK_WALL_Z + 0.02),
        Vec3::new((last - first) * 0.5 + 0.5, 0.07, 0.02),
    );
    for (i, h) in l.hooks.iter().enumerate() {
        let wall = Vec3::new(h.hook.x, h.hook.y + 0.04, BACK_WALL_Z + 0.04);
        // Plate, arm out to the rim, the coated curl that holds it.
        p.s("steel_dark")
            .cuboid(wall, Vec3::new(0.035, 0.05, 0.008), Quat::IDENTITY);
        p.s("steel").sweep(
            &[
                wall,
                h.hook + Vec3::new(0.0, 0.03, -0.08),
                h.hook + Vec3::new(0.0, 0.01, 0.0),
                h.hook + Vec3::new(0.0, -0.035, 0.04),
                h.hook + Vec3::new(0.0, -0.06, 0.035),
            ],
            &[0.007; 5],
            6,
            true,
        );
        p.s("rubber").sweep(
            &[
                h.hook + Vec3::new(0.0, 0.012, -0.03),
                h.hook + Vec3::new(0.0, -0.035, 0.04),
                h.hook + Vec3::new(0.0, -0.062, 0.035),
            ],
            &[0.0095; 3],
            6,
            true,
        );
        // Name card plate on the wall under the bike.
        p.s("steel_dark").cuboid(
            h.card - Vec3::Z * 0.002,
            Vec3::new(0.2, 0.075, 0.004),
            Quat::IDENTITY,
        );
        p.s("card").cuboid(
            h.card + Vec3::Z * 0.003,
            Vec3::new(0.185, 0.062, 0.003),
            Quat::IDENTITY,
        );
        if i == plus {
            // The empty hook: a "+" sign on its plate (a shape, not text).
            for half in [
                Vec3::new(0.045, 0.012, 0.003),
                Vec3::new(0.012, 0.045, 0.003),
            ] {
                p.s("wall_accent")
                    .cuboid(h.card + Vec3::Z * 0.008, half, Quat::IDENTITY);
            }
        }
    }
    p
}

/// Something on the left wall: `along` is z, `out` is x from the wall.
fn left(l: &Layout, out: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(-l.half_width() + out, y, z)
}

fn bench(l: &Layout) -> Mats {
    let mut p = Mats::default();
    let c = l.bench;
    let (hx, hz) = (0.35, 1.0);
    // Worktop, legs, lower shelf, drawers.
    p.box_("wood", c - Vec3::Y * 0.025, Vec3::new(hx, 0.025, hz));
    for (sx, sz) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
        p.box_(
            "steel_dark",
            Vec3::new(
                c.x + sx * (hx - 0.04),
                (c.y - 0.05) * 0.5,
                c.z + sz * (hz - 0.04),
            ),
            Vec3::new(0.025, (c.y - 0.05) * 0.5, 0.025),
        );
    }
    p.box_(
        "wood_dark",
        Vec3::new(c.x, 0.2, c.z),
        Vec3::new(hx - 0.04, 0.015, hz - 0.04),
    );
    for k in 0..3 {
        let z = c.z - 0.6 + k as f32 * 0.6;
        p.box_(
            "steel_dark",
            Vec3::new(c.x + 0.08, c.y - 0.16, z),
            Vec3::new(hx - 0.1, 0.1, 0.27),
        );
        p.box_(
            "steel",
            Vec3::new(c.x + hx - 0.015, c.y - 0.16, z),
            Vec3::new(0.012, 0.012, 0.08),
        );
    }
    // A vise at the front end, a parts tray and a bottle of chain oil.
    let vise = c + Vec3::new(0.12, 0.0, 0.75);
    p.box_(
        "tool_blue",
        vise + Vec3::Y * 0.04,
        Vec3::new(0.07, 0.04, 0.1),
    );
    p.box_(
        "tool_blue",
        vise + Vec3::new(0.0, 0.1, -0.06),
        Vec3::new(0.08, 0.03, 0.015),
    );
    p.box_(
        "tool_blue",
        vise + Vec3::new(0.0, 0.1, 0.04),
        Vec3::new(0.08, 0.03, 0.015),
    );
    p.s("steel").tube(
        vise + Vec3::new(0.0, 0.06, 0.1),
        vise + Vec3::new(0.0, 0.06, 0.2),
        0.008,
        0.008,
        6,
        true,
    );
    p.box_(
        "bin_grey",
        c + Vec3::new(-0.05, 0.03, -0.3),
        Vec3::new(0.15, 0.03, 0.1),
    );
    p.s("tool_yellow").tube(
        c + Vec3::new(0.1, 0.0, -0.75),
        c + Vec3::new(0.1, 0.14, -0.75),
        0.03,
        0.03,
        10,
        true,
    );
    p.s("tool_red").tube(
        c + Vec3::new(0.1, 0.14, -0.75),
        c + Vec3::new(0.1, 0.19, -0.75),
        0.012,
        0.004,
        8,
        true,
    );
    // Parts shelf at the front of the left wall: four boards with boxes
    // and bins.
    let z0 = 1.15;
    for k in 0..2 {
        p.box_(
            "steel_dark",
            left(l, 0.2, 0.95, z0 + if k == 0 { -0.6 } else { 0.6 }),
            Vec3::new(0.2, 0.95, 0.015),
        );
    }
    for (k, y) in [0.08f32, 0.55, 1.05, 1.55].into_iter().enumerate() {
        p.box_("wood", left(l, 0.2, y, z0), Vec3::new(0.2, 0.015, 0.6));
        let mut z = z0 - 0.55;
        let mut j = 0usize;
        while z < z0 + 0.45 {
            let w = 0.08 + 0.03 * ((k * 3 + j * 7) % 4) as f32;
            let h = 0.07 + 0.04 * ((k + j * 5) % 3) as f32;
            let mat = match (k + j) % 4 {
                0 | 2 => "cardboard",
                1 => "bin_blue",
                _ => "bin_grey",
            };
            p.box_(
                mat,
                left(l, 0.18, y + 0.015 + h, z + w),
                Vec3::new(0.15, h, w),
            );
            z += 2.0 * w + 0.04;
            j += 1;
        }
    }
    p
}

fn pegboard(l: &Layout) -> Mats {
    let mut p = Mats::default();
    let (z0, z1, y0, y1) = (l.bench.z - 0.9, l.bench.z + 0.9, 1.08, 2.25);
    let (zc, yc) = ((z0 + z1) * 0.5, (y0 + y1) * 0.5);
    let face = 0.03;
    p.box_(
        "pegboard",
        left(l, 0.015, yc, zc),
        Vec3::new(0.015, (y1 - y0) * 0.5, (z1 - z0) * 0.5),
    );
    p.box_(
        "wood_dark",
        left(l, 0.02, y1 + 0.02, zc),
        Vec3::new(0.02, 0.02, (z1 - z0) * 0.5 + 0.02),
    );
    p.box_(
        "wood_dark",
        left(l, 0.02, y0 - 0.02, zc),
        Vec3::new(0.02, 0.02, (z1 - z0) * 0.5 + 0.02),
    );
    // Holes (a grid of dark dots).
    let mut y = y0 + 0.06;
    while y < y1 - 0.03 {
        let mut z = z0 + 0.06;
        while z < z1 - 0.03 {
            p.box_(
                "peg_hole",
                left(l, face + 0.0005, y, z),
                Vec3::new(0.0006, 0.005, 0.005),
            );
            z += 0.1;
        }
        y += 0.1;
    }
    let x = face + 0.012;
    let peg = |p: &mut Mats, y: f32, z: f32| {
        p.s("steel").tube(
            left(l, face, y, z),
            left(l, face + 0.05, y + 0.01, z),
            0.003,
            0.003,
            4,
            false,
        );
    };
    // Wrenches, short to long: shaft, ring end and open end.
    for k in 0..6 {
        let z = z0 + 0.12 + k as f32 * 0.07;
        let len = 0.13 + 0.025 * k as f32;
        let top = y1 - 0.12;
        let r = 0.016 + 0.002 * k as f32;
        p.s("steel").cuboid(
            left(l, x, top - len * 0.5, z),
            Vec3::new(0.003, len * 0.5, 0.006 + 0.0006 * k as f32),
            Quat::IDENTITY,
        );
        p.s("steel")
            .annulus(left(l, x, top + r * 0.5, z), Vec3::X, r * 0.5, r, 0.006, 12);
        for dz in [-1.0f32, 1.0] {
            p.s("steel").cuboid(
                left(l, x, top - len - r * 0.4, z + dz * r * 0.6),
                Vec3::new(0.003, r * 0.6, r * 0.25),
                Quat::IDENTITY,
            );
        }
        peg(&mut p, top + r * 0.5, z);
    }
    // Screwdrivers: coloured handle, steel shaft.
    for (k, mat) in [
        "tool_red",
        "tool_yellow",
        "tool_blue",
        "tool_red",
        "tool_yellow",
    ]
    .into_iter()
    .enumerate()
    {
        let z = z0 + 0.6 + k as f32 * 0.06;
        let top = y1 - 0.1;
        p.s(mat).tube(
            left(l, x + 0.01, top, z),
            left(l, x + 0.01, top - 0.1, z),
            0.013,
            0.011,
            8,
            true,
        );
        p.s("steel").tube(
            left(l, x + 0.01, top - 0.1, z),
            left(l, x + 0.01, top - 0.22 - 0.02 * (k % 2) as f32, z),
            0.003,
            0.003,
            5,
            true,
        );
    }
    // Hammer, pliers, hex keys, tyre levers, a chain tool.
    let hz = z0 + 1.05;
    p.s("wood").tube(
        left(l, x + 0.01, y1 - 0.1, hz),
        left(l, x + 0.01, y1 - 0.42, hz),
        0.012,
        0.014,
        8,
        true,
    );
    p.s("steel").cuboid(
        left(l, x + 0.01, y1 - 0.09, hz),
        Vec3::new(0.015, 0.018, 0.06),
        Quat::IDENTITY,
    );
    let pz = z0 + 1.25;
    for side in [-1.0f32, 1.0] {
        p.s("tool_red").sweep(
            &[
                left(l, x, y1 - 0.12, pz),
                left(l, x, y1 - 0.2, pz + side * 0.015),
                left(l, x, y1 - 0.32, pz + side * 0.03),
            ],
            &[0.007, 0.008, 0.008],
            6,
            true,
        );
    }
    p.s("steel").cuboid(
        left(l, x, y1 - 0.1, pz),
        Vec3::new(0.005, 0.03, 0.012),
        Quat::IDENTITY,
    );
    for k in 0..7 {
        let z = z0 + 1.4 + k as f32 * 0.025;
        let len = 0.06 + 0.012 * k as f32;
        p.s("steel_dark").cuboid(
            left(l, x, y1 - 0.12 - len * 0.5, z),
            Vec3::new(0.003, len * 0.5, 0.003),
            Quat::IDENTITY,
        );
        p.s("steel_dark").cuboid(
            left(l, x + 0.015, y1 - 0.12, z),
            Vec3::new(0.015, 0.003, 0.003),
            Quat::IDENTITY,
        );
    }
    for (k, mat) in ["tool_blue", "tool_yellow", "tool_blue"]
        .into_iter()
        .enumerate()
    {
        let z = z0 + 0.25 + k as f32 * 0.035;
        p.s(mat).cuboid(
            left(l, x, y0 + 0.3, z),
            Vec3::new(0.004, 0.06, 0.012),
            Quat::IDENTITY,
        );
    }
    peg(&mut p, y0 + 0.37, z0 + 0.285);
    // A spare tyre hung on a peg, and a cassette tool.
    p.s("rubber").torus(
        left(l, x + 0.02, y0 + 0.35, z0 + 0.95),
        Vec3::X,
        0.2,
        0.014,
        24,
        6,
        0.0,
    );
    peg(&mut p, y0 + 0.55, z0 + 0.95);
    p.s("steel_dark").tube(
        left(l, x, y0 + 0.2, z0 + 1.45),
        left(l, x, y0 + 0.45, z0 + 1.45),
        0.009,
        0.009,
        6,
        true,
    );
    p.s("steel").annulus(
        left(l, x, y0 + 0.47, z0 + 1.45),
        Vec3::X,
        0.008,
        0.02,
        0.012,
        12,
    );
    p
}

fn mirror_corner(l: &Layout) -> Mats {
    let mut p = Mats::default();
    let w = l.half_width();
    let mc = l.mirror;
    // Mirror in a wooden frame, facing −X.
    p.box_(
        "wood_dark",
        mc + Vec3::new(-0.02, 0.0, 0.0),
        Vec3::new(0.02, 0.95, 0.5),
    );
    p.box_(
        "mirror",
        mc + Vec3::new(-0.045, 0.0, 0.0),
        Vec3::new(0.004, 0.88, 0.43),
    );
    // Bench in front of the mirror's right.
    let b = Vec3::new(w - 0.32, 0.0, mc.z + 1.15);
    p.box_("wood", b + Vec3::Y * 0.44, Vec3::new(0.2, 0.025, 0.55));
    for dz in [-0.48f32, 0.48] {
        for dx in [-0.15f32, 0.15] {
            p.box_(
                "steel_dark",
                b + Vec3::new(dx, 0.21, dz),
                Vec3::new(0.018, 0.21, 0.018),
            );
        }
    }
    // A pair of shoes under the bench and a towel on it.
    for dz in [-0.08f32, 0.08] {
        p.s("card").ellipsoid(
            b + Vec3::new(-0.05, 0.04, dz + 0.1),
            Vec3::new(0.12, 0.04, 0.045),
            Quat::from_rotation_y(FRAC_PI_2),
            5,
            10,
        );
    }
    p.box_(
        "bin_blue",
        b + Vec3::new(0.0, 0.48, -0.3),
        Vec3::new(0.15, 0.015, 0.12),
    );
    // Locker (two doors, handles, vents), with a kit hook rail beside it.
    let lk = Vec3::new(w - 0.3, 1.0, -1.65);
    p.box_("locker", lk, Vec3::new(0.29, 1.0, 0.4));
    for dz in [-0.2f32, 0.2] {
        p.box_(
            "steel_dark",
            lk + Vec3::new(-0.292, 0.0, 0.0),
            Vec3::new(0.002, 0.95, 0.004),
        );
        p.box_(
            "steel",
            lk + Vec3::new(-0.3, 0.1, dz - dz.signum() * 0.15),
            Vec3::new(0.012, 0.06, 0.01),
        );
        for k in 0..4 {
            p.box_(
                "steel_dark",
                lk + Vec3::new(-0.292, 0.75 + k as f32 * 0.04, dz),
                Vec3::new(0.002, 0.008, 0.12),
            );
        }
    }
    p.box_(
        "steel_dark",
        lk + Vec3::new(-0.292, 0.0, 0.0),
        Vec3::new(0.003, 0.97, 0.004),
    );
    p.box_(
        "steel_dark",
        lk + Vec3::new(0.0, -0.97, 0.0),
        Vec3::new(0.27, 0.03, 0.38),
    );
    // Kit hanging on the rail between locker and window: a jersey shape.
    let rail = Vec3::new(w - 0.06, 1.95, -1.05);
    p.s("steel").tube(
        rail - Vec3::Z * 0.15,
        rail + Vec3::Z * 0.15,
        0.008,
        0.008,
        6,
        true,
    );
    p.s("tool_red").cuboid(
        rail + Vec3::new(-0.04, -0.28, 0.0),
        Vec3::new(0.02, 0.26, 0.17),
        Quat::IDENTITY,
    );
    p.s("tool_red").cuboid(
        rail + Vec3::new(-0.04, -0.08, 0.0),
        Vec3::new(0.02, 0.06, 0.25),
        Quat::IDENTITY,
    );
    p
}

fn window(l: &Layout) -> Mats {
    let mut p = Mats::default();
    let (c, size) = l.window;
    let (hw, hh) = (size.x * 0.5, size.y * 0.5);
    p.box_(
        "window_glass",
        c + Vec3::new(-0.003, 0.0, 0.0),
        Vec3::new(0.003, hh, hw),
    );
    for dy in [-1.0f32, 1.0] {
        p.box_(
            "window_frame",
            c + Vec3::new(-0.02, dy * (hh + 0.03), 0.0),
            Vec3::new(0.025, 0.03, hw + 0.06),
        );
    }
    for dz in [-1.0f32, 0.0, 1.0] {
        p.box_(
            "window_frame",
            c + Vec3::new(-0.02, 0.0, dz * (hw + 0.03)),
            Vec3::new(0.025, hh, if dz == 0.0 { 0.015 } else { 0.03 }),
        );
    }
    p.box_(
        "window_frame",
        c + Vec3::new(-0.03, 0.0, 0.0),
        Vec3::new(0.012, 0.012, hw),
    );
    // Sill with a plant pot.
    p.box_(
        "window_frame",
        c + Vec3::new(-0.08, -hh - 0.07, 0.0),
        Vec3::new(0.08, 0.015, hw + 0.08),
    );
    let pot = c + Vec3::new(-0.09, -hh - 0.055, hw * 0.5);
    p.s("cardboard")
        .tube(pot, pot + Vec3::Y * 0.1, 0.045, 0.055, 10, true);
    p.s("wall_accent").ellipsoid(
        pot + Vec3::Y * 0.16,
        Vec3::new(0.07, 0.07, 0.07),
        Quat::IDENTITY,
        6,
        10,
    );
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose::bike_nodes::LOD1;
    use ocw_core::ProfileId;

    /// World positions of a model's near-LOD vertices posed at `(t, r)`.
    fn posed(m: &Model, (t, r): (Vec3, Quat)) -> Vec<Vec3> {
        let far = m.node(LOD1);
        let mut out = Vec::new();
        for (i, n) in m.nodes.iter().enumerate() {
            if far.is_some_and(|f| m.is_below(i, f)) {
                continue;
            }
            let w = m.world(i);
            for prim in &n.primitives {
                out.extend(
                    prim.positions
                        .iter()
                        .map(|p| t + r * w.transform_point3(Vec3::from(*p))),
                );
            }
        }
        out
    }

    #[test]
    fn the_room_has_its_nodes_within_the_triangle_budget() {
        for bikes in [1, 6, 24] {
            let l = layout(bikes);
            let m = room(&l);
            for n in NODES {
                let i = m.node(n).unwrap_or_else(|| panic!("node {n}"));
                assert!(m.triangles_below(Some(i)) > 0, "{n} is empty");
            }
            assert!(m.triangles() < 40_000, "{bikes}: {}", m.triangles());
            // Every material is one of ours, and named for its slot.
            for mat in &m.materials {
                assert!(
                    MATERIALS.iter().any(|(n, ..)| *n == mat.name),
                    "{}",
                    mat.name
                );
            }
        }
    }

    #[test]
    fn normals_are_unit_and_agree_with_the_winding() {
        let m = room(&layout(6));
        for n in &m.nodes {
            for p in &n.primitives {
                for t in p.indices.chunks(3) {
                    let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(p.positions[t[k] as usize]));
                    let face = (b - a).cross(c - a);
                    if face.length() < 1e-12 {
                        continue;
                    }
                    for k in t {
                        let nn = Vec3::from(p.normals[*k as usize]);
                        assert!((nn.length() - 1.0).abs() < 1e-3, "{}: {nn}", n.name);
                        assert!(nn.dot(face) > -1e-9, "{}: inverted face", n.name);
                    }
                }
            }
        }
    }

    #[test]
    fn the_walls_face_into_the_room() {
        // The faces of the walls node that bound the room look inwards.
        let l = layout(6);
        let m = room(&l);
        let walls = &m.nodes[m.node("walls").unwrap()];
        let (lo, hi) = l.bounds;
        let centre = Vec3::new(0.0, hi.y * 0.5, (lo.z + hi.z) * 0.5);
        let mut inward = 0;
        for p in &walls.primitives {
            for t in p.indices.chunks(3) {
                let a = Vec3::from(p.positions[t[0] as usize]);
                let n = Vec3::from(p.normals[t[0] as usize]);
                // A face lying on the back wall's inner plane.
                if (a.z - lo.z).abs() < 1e-4 && n.z > 0.9 {
                    inward += 1;
                    assert!(n.dot(centre - a) > 0.0);
                }
            }
        }
        assert!(inward >= 2, "the back wall's inner face");
    }

    #[test]
    fn hooks_are_evenly_spaced_on_the_back_wall_inside_the_room() {
        for bikes in [1, 6, 24, 40] {
            let l = layout(bikes);
            assert_eq!(l.hooks.len(), bikes.min(24) + 1);
            let (lo, hi) = l.bounds;
            for w in l.hooks.windows(2) {
                assert!((w[1].hook.x - w[0].hook.x - HOOK_SPACING).abs() < 1e-4);
                assert_eq!(w[0].hook.y, w[1].hook.y, "same height");
            }
            let mid = (l.hooks[0].hook.x + l.hooks.last().unwrap().hook.x) * 0.5;
            assert!(mid.abs() < 1e-4, "centred");
            for h in &l.hooks {
                assert!(
                    h.hook.x - HOOK_SPACING * 0.5 > lo.x && h.hook.x + HOOK_SPACING * 0.5 < hi.x
                );
                assert!((lo.z..lo.z + 1.0).contains(&h.hook.z));
                assert!((lo.z..lo.z + 0.05).contains(&h.card.z));
                assert!(h.card.y < h.hook.y);
            }
        }
    }

    #[test]
    fn the_stand_bike_floats_with_its_drive_side_to_the_camera() {
        let l = layout(6);
        let (t, r) = l.stand_bike;
        // The bike's right (drive side) faces the camera, its front +X.
        assert!((r * Vec3::X).dot(Vec3::Z) > 0.99);
        assert!((r * Vec3::NEG_Z).dot(Vec3::X) > 0.99);
        let (lo, hi) = l.bounds;
        for p in ProfileId::ALL {
            let pts = posed(crate::shipped::bike_model(p), (t, r));
            let min_y = pts.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
            assert!(min_y >= STAND_LIFT - 0.01, "{p:?}: wheels at {min_y}");
            for v in &pts {
                assert!(
                    v.x > lo.x && v.x < hi.x && v.z > lo.z && v.y < hi.y,
                    "{p:?} {v}"
                );
            }
            // The clamp is close to the seat post of every bike.
            let rig = crate::shipped::bike_rig(p);
            let saddle = t + r * rig.saddle;
            let d = saddle.distance(l.stand_clamp);
            assert!(d < 0.3, "{p:?}: clamp {d} m from the saddle");
        }
        assert!(l.stand_clamp.y > l.stand_base.y + 0.8);
        assert!(l.spot.y > l.stand_clamp.y);
    }

    #[test]
    fn hung_bikes_stay_in_the_room_and_clear_each_other() {
        let l = layout(6);
        let (lo, hi) = l.bounds;
        // For every bike: how far it reaches left and right of its hook.
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for p in ProfileId::ALL {
            let rig = crate::shipped::bike_rig(p);
            for h in &l.hooks {
                let pose = h.bike_pose(rig.wheel_radius, rig.wheelbase);
                let pts = posed(crate::shipped::bike_model(p), pose);
                for v in &pts {
                    assert!(v.z > lo.z + 0.02, "{p:?} behind the wall: {v}");
                    assert!(v.y > 0.1 && v.y < hi.y - 0.05, "{p:?} {v}");
                    assert!(v.x > lo.x && v.x < hi.x, "{p:?} {v}");
                    left = left.max(h.hook.x - v.x);
                    right = right.max(v.x - h.hook.x);
                }
                // The front rim's top is at the hook.
                let front_hub =
                    pose.0 + pose.1 * Vec3::new(0.0, rig.wheel_radius, -rig.wheelbase * 0.5);
                assert!(
                    (front_hub + Vec3::Y * (rig.wheel_radius - RIM_INSET)).distance(h.hook) < 1e-4
                );
                // Under the card? No: the card is below the bike.
                let min_y = pts.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
                assert!(
                    min_y > h.card.y + 0.05,
                    "{p:?}: bike over its card ({min_y})"
                );
            }
        }
        assert!(
            left + right < HOOK_SPACING - 0.02,
            "neighbours overlap: {left} + {right}"
        );
    }

    #[test]
    fn suggested_views_look_into_the_room() {
        let l = layout(6);
        let (lo, hi) = l.bounds;
        let mut views = vec![
            l.overview_view(),
            l.stand_view(),
            l.mirror_view(),
            l.bench_view(),
        ];
        views.extend((0..l.hooks.len()).map(|i| l.wall_view(i)));
        for v in views {
            assert!(
                v.eye.x > lo.x && v.eye.x < hi.x && v.eye.y > 0.3 && v.eye.y < hi.y,
                "{v:?}"
            );
            assert!(v.eye.z > lo.z && v.eye.z <= hi.z);
            assert!(v.eye.distance(v.target) > 0.5);
        }
    }
}
