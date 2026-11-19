//! The workshop room (spec §14 Phase 4, 4A *Rendering*, *Layout*): a
//! bike workshop, own work (MIT OR Apache-2.0), no brands and no text.
//! Concrete floor, light walls and a ceiling with panel lamps, the
//! **repair stand** in the foreground centre, the **back wall** with one
//! hook per owned bike (plus an empty "+" hook at the end), the
//! **workbench** with a pegboard of tools and a parts shelf on the left,
//! the **changing corner** (mirror, bench, locker) and a window on the
//! right.
//!
//! Coordinates are the app's: metres, Y up, the floor at `y = 0`, the back
//! wall at [`BACK_WALL_Z`], the camera on the +Z side (the room has no
//! front wall). [`layout`] places everything; the bikes are not part of the
//! room (the renderer puts the bike models at [`Layout::stand_bike`] and
//! [`Hook::bike_pose`]).
//!
//! **The pieces are model files** (spec §14 Phase 15, M3 / 15H):
//! `artwork/3dmodels/workshop/*.glb`, one per [`Piece`]; [`room`] places
//! them for the layout – the fixed pieces where the layout puts them, the
//! room's slabs (floor, walls, ceiling, skirting, the hook rail) as unit
//! pieces scaled to the room's size for any number of bikes. The mirror's
//! reflection stays code (the renderer's mirror camera at
//! [`Layout::mirror_glass`], the file's `anchor_glass`). [`Piece::model`]
//! is the generator that wrote the files (Q6: the files are the source;
//! the generator stays the exporter until all generators are removed in
//! one step).
//!
//! The materials ([`MATERIALS`]) are paint slots that are never
//! recoloured. `lamp` and `window_glass` are meant to be drawn emissive
//! (light sources seen in the room).

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
        bench: Vec3::new(-half + 0.4, BENCH_Y, BENCH_Z),
        mirror: Vec3::new(half - 0.04, MIRROR_Y, 0.9),
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

    /// The helmet's hook (spec §14 Phase 4, 4N Q28): on the mirror's wall
    /// between the window and the mirror, at head height; the helmet
    /// hangs from it while the character editor shows the bare head.
    pub fn helmet_hook(&self) -> Vec3 {
        let (glass, half) = self.mirror_glass();
        let window_edge = self.window.0.z + self.window.1.x * 0.5;
        let z = 0.5 * (window_edge + glass.z - half.x);
        Vec3::new(self.half_width() - 0.06, 1.55, z)
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

    /// Where the rider stands in the changing corner (spec §14 Phase 4,
    /// 4C): the feet on the floor 1.15 m in front of the mirror, facing it
    /// (the rider model faces −Z; turned to +X).
    pub fn mirror_rider(&self) -> (Vec3, Quat) {
        (
            Vec3::new(self.mirror.x - 1.15, 0.0, self.mirror.z),
            Quat::from_rotation_y(-FRAC_PI_2),
        )
    }

    /// The mirror's glass: the centre of its front face and its half size
    /// (along the wall, z; up, y). It faces −X.
    pub fn mirror_glass(&self) -> (Vec3, Vec2) {
        (
            Vec3::new(self.mirror.x + MIRROR_GLASS_X, self.mirror.y, self.mirror.z),
            Vec2::new(0.43, 0.88),
        )
    }

    /// The character editor's view: over the rider's shoulder into the
    /// mirror, the rider's back left of the picture and the reflection in
    /// the middle; `front`: straight at the rider from the mirror's side
    /// (low graphics quality: no reflection).
    pub fn dress_view(&self, front: bool) -> View {
        let (feet, _) = self.mirror_rider();
        if front {
            View {
                eye: feet + Vec3::new(1.0, 1.45, 0.35),
                target: feet + Vec3::new(0.0, 1.05, 0.12),
            }
        } else {
            View {
                eye: feet + Vec3::new(-2.05, 1.95, 0.78),
                target: Vec3::new(self.mirror.x - 0.45, 1.05, self.mirror.z - 0.08),
            }
        }
    }

    /// *Try on the bike*: the stand bike with the rider on it, from its
    /// drive side.
    pub fn try_on_view(&self) -> View {
        let c = self.stand_bike.0 + Vec3::Y * 0.85;
        View {
            eye: c + Vec3::new(0.55, 0.35, 2.6),
            target: c,
        }
    }

    /// The rider's hotspots standing in the changing corner (head, torso,
    /// hands, legs, feet), for a rider `scale` times the default height.
    pub fn rider_hotspots(&self, scale: f32) -> [Vec3; 5] {
        let (feet, rot) = self.mirror_rider();
        [
            Vec3::new(0.0, 1.68, -0.02),
            Vec3::new(0.0, 1.3, 0.0),
            Vec3::new(0.23, 0.86, -0.02),
            Vec3::new(-0.09, 0.6, 0.0),
            Vec3::new(0.09, 0.06, -0.06),
        ]
        .map(|p| feet + rot * (p * scale))
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

    /// The shapes as primitives of node `node`.
    fn put(self, model: &mut Model, node: usize) {
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
                ..Default::default()
            });
        }
    }
}

/// The room's node names: the groups [`room`] puts the pieces in.
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

/// Wall thickness (the walls and the ceiling are slabs this thick).
const WALL_T: f32 = 0.1;
/// The parts shelf's centre along the left wall (z).
const SHELF_Z: f32 = 1.15;
/// The floor pump beside the stand.
const PUMP: Vec3 = Vec3::new(1.15, 0.0, STAND_Z - 0.35);

/// A piece of the workshop: one model file in `artwork/3dmodels/workshop/`
/// (spec §14 Phase 15, M3 / 15H). [`room`] places the pieces for a layout:
/// fixed pieces where the layout puts them, the room's slabs (floor,
/// walls, ceiling, skirting, the hook rail) as unit pieces 1 m long,
/// scaled to the room's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Piece {
    /// The concrete slab, 1 × 1 m (x, z), its top at the origin.
    FloorSegment,
    /// An expansion joint of the floor, 1 m along x.
    FloorJoint,
    /// The rubber mat under the stand.
    WorkMat,
    /// The floor pump beside the stand.
    FloorPump,
    /// A wall slab, 1 m wide (x) and 1 m high from its foot, 0.2 m thick.
    WallSegment,
    /// The painted dado of the back wall, 1 m along x, on the wall's face.
    WallDado,
    /// A skirting board, 1 m along x, on the wall's face.
    Skirting,
    /// The ceiling slab, 1 × 1 m, its underside at the origin.
    CeilingSegment,
    /// A ceiling panel lamp, hanging from the origin.
    CeilingLamp,
    /// The repair stand (tripod, post, clamp, tool tray); origin on the
    /// floor at the post's foot, `anchor_clamp` at the clamp.
    Stand,
    /// The wooden rail of the hook wall, 1 m along x, on the wall's face.
    HookRail,
    /// One bike hook with its name card on the wall; `plus_sign` (the
    /// empty hook's "+") only on the last hook; `anchor_hook` where it
    /// holds the rim, `anchor_card` the card's centre.
    Hook,
    /// The workbench with drawers, vise, tray and oil bottle.
    Workbench,
    /// The parts shelf on the left wall with its boxes and bins.
    PartsShelf,
    /// The pegboard of tools above the workbench.
    Pegboard,
    /// The mirror in its frame on the right wall; `anchor_glass` is the
    /// glass's centre (the reflection's plane).
    Mirror,
    /// The bench in front of the mirror, with shoes and a towel.
    ChangingBench,
    /// The two-door locker.
    Locker,
    /// The kit rail with a jersey.
    KitRail,
    /// The window with its frame, sill and a plant pot.
    Window,
}

impl Piece {
    pub const ALL: [Piece; 20] = [
        Piece::FloorSegment,
        Piece::FloorJoint,
        Piece::WorkMat,
        Piece::FloorPump,
        Piece::WallSegment,
        Piece::WallDado,
        Piece::Skirting,
        Piece::CeilingSegment,
        Piece::CeilingLamp,
        Piece::Stand,
        Piece::HookRail,
        Piece::Hook,
        Piece::Workbench,
        Piece::PartsShelf,
        Piece::Pegboard,
        Piece::Mirror,
        Piece::ChangingBench,
        Piece::Locker,
        Piece::KitRail,
        Piece::Window,
    ];

    /// The piece's file (15H: function first, then shape).
    pub fn file(self) -> &'static str {
        match self {
            Piece::FloorSegment => "workshop/floor-segment.glb",
            Piece::FloorJoint => "workshop/floor-joint-strip.glb",
            Piece::WorkMat => "workshop/mat-rubber-work.glb",
            Piece::FloorPump => "workshop/pump-floor-track.glb",
            Piece::WallSegment => "workshop/wall-segment.glb",
            Piece::WallDado => "workshop/wall-dado-panel.glb",
            Piece::Skirting => "workshop/skirting-board.glb",
            Piece::CeilingSegment => "workshop/ceiling-segment.glb",
            Piece::CeilingLamp => "workshop/lamp-panel-ceiling.glb",
            Piece::Stand => "workshop/stand-repair-clamp.glb",
            Piece::HookRail => "workshop/rail-hook-wood.glb",
            Piece::Hook => "workshop/hook-wall-bike.glb",
            Piece::Workbench => "workshop/bench-work-wood.glb",
            Piece::PartsShelf => "workshop/shelf-parts-boxes.glb",
            Piece::Pegboard => "workshop/wall-tool-pegboard.glb",
            Piece::Mirror => "workshop/mirror-wall-framed.glb",
            Piece::ChangingBench => "workshop/bench-changing-wood.glb",
            Piece::Locker => "workshop/locker-double-steel.glb",
            Piece::KitRail => "workshop/rail-kit-jersey.glb",
            Piece::Window => "workshop/window-sill-plant.glb",
        }
    }

    /// The model id of the piece's file.
    pub fn id(self) -> &'static str {
        crate::kinds::id_of(self.file())
    }

    /// The piece's root node.
    pub fn node(self) -> &'static str {
        match self {
            Piece::FloorSegment => "floor_segment",
            Piece::FloorJoint => "floor_joint",
            Piece::WorkMat => "work_mat",
            Piece::FloorPump => "floor_pump",
            Piece::WallSegment => "wall_segment",
            Piece::WallDado => "wall_dado",
            Piece::Skirting => "skirting",
            Piece::CeilingSegment => "ceiling_segment",
            Piece::CeilingLamp => "ceiling_lamp",
            Piece::Stand => "stand",
            Piece::HookRail => "hook_rail",
            Piece::Hook => "hook",
            Piece::Workbench => "workbench",
            Piece::PartsShelf => "parts_shelf",
            Piece::Pegboard => "pegboard",
            Piece::Mirror => "mirror",
            Piece::ChangingBench => "changing_bench",
            Piece::Locker => "locker",
            Piece::KitRail => "kit_rail",
            Piece::Window => "window",
        }
    }

    /// The generator of the piece's file (spec §14 Phase 15 Q6: the file is
    /// the source; this code stays the exporter until all generators are
    /// removed in one step). The apps read the files ([`room`]).
    pub fn model(self) -> Model {
        let mut m = Model::default();
        let root = m.add(Node::new(self.node(), None, Vec3::ZERO));
        let l = layout(0);
        match self {
            Piece::FloorSegment => {
                let mut p = Mats::default();
                p.box_(
                    "concrete",
                    Vec3::new(0.0, -0.05, 0.0),
                    Vec3::new(0.5, 0.05, 0.5),
                );
                p.put(&mut m, root);
            }
            Piece::FloorJoint => {
                let mut p = Mats::default();
                p.box_(
                    "concrete_dark",
                    Vec3::new(0.0, 0.0005, 0.0),
                    Vec3::new(0.5, 0.0006, 0.006),
                );
                p.put(&mut m, root);
            }
            Piece::WorkMat => {
                let mut p = Mats::default();
                p.box_(
                    "rubber",
                    Vec3::new(0.0, 0.004, 0.0),
                    Vec3::new(1.1, 0.004, 0.85),
                );
                p.put(&mut m, root);
            }
            Piece::FloorPump => floor_pump().put(&mut m, root),
            Piece::WallSegment => {
                let mut p = Mats::default();
                p.box_(
                    "wall",
                    Vec3::new(0.0, 0.5, 0.0),
                    Vec3::new(0.5, 0.5, WALL_T),
                );
                p.put(&mut m, root);
            }
            Piece::WallDado => {
                let mut p = Mats::default();
                p.box_(
                    "wall_accent",
                    Vec3::new(0.0, 0.6, 0.002),
                    Vec3::new(0.5, 0.6, 0.002),
                );
                p.put(&mut m, root);
            }
            Piece::Skirting => {
                let mut p = Mats::default();
                p.box_(
                    "skirting",
                    Vec3::new(0.0, 0.05, 0.008),
                    Vec3::new(0.5, 0.05, 0.008),
                );
                p.put(&mut m, root);
            }
            Piece::CeilingSegment => {
                let mut p = Mats::default();
                p.box_(
                    "ceiling",
                    Vec3::new(0.0, WALL_T, 0.0),
                    Vec3::new(0.5, WALL_T, 0.5),
                );
                p.put(&mut m, root);
            }
            Piece::CeilingLamp => {
                let mut p = Mats::default();
                p.box_(
                    "steel_dark",
                    Vec3::new(0.0, -0.02, 0.0),
                    Vec3::new(0.62, 0.02, 0.12),
                );
                p.box_(
                    "lamp",
                    Vec3::new(0.0, -0.041, 0.0),
                    Vec3::new(0.58, 0.002, 0.09),
                );
                p.put(&mut m, root);
            }
            Piece::Stand => {
                let clamp = l.stand_clamp - l.stand_base;
                stand(clamp).put(&mut m, root);
                m.add(Node::new("anchor_clamp", Some(root), clamp));
            }
            Piece::HookRail => {
                let mut p = Mats::default();
                p.box_(
                    "wood",
                    Vec3::new(0.0, HOOK_Y + 0.04, 0.02),
                    Vec3::new(0.5, 0.07, 0.02),
                );
                p.put(&mut m, root);
            }
            Piece::Hook => {
                let (hook, card) = hook_points();
                let (body, plus) = hook_piece();
                body.put(&mut m, root);
                let sign = m.add(Node::new("plus_sign", Some(root), Vec3::ZERO));
                plus.put(&mut m, sign);
                m.add(Node::new("anchor_hook", Some(root), hook));
                m.add(Node::new("anchor_card", Some(root), card));
            }
            Piece::Workbench => workbench().put(&mut m, root),
            Piece::PartsShelf => parts_shelf().put(&mut m, root),
            Piece::Pegboard => pegboard().put(&mut m, root),
            Piece::Mirror => {
                mirror().put(&mut m, root);
                m.add(Node::new(
                    "anchor_glass",
                    Some(root),
                    Vec3::new(MIRROR_GLASS_X, l.mirror.y, 0.0),
                ));
            }
            Piece::ChangingBench => changing_bench().put(&mut m, root),
            Piece::Locker => locker().put(&mut m, root),
            Piece::KitRail => kit_rail().put(&mut m, root),
            Piece::Window => window(l.window.0.y, l.window.1).put(&mut m, root),
        }
        m
    }
}

/// The glass's face in front of the mirror's origin (x, towards −X).
const MIRROR_GLASS_X: f32 = -0.049;

/// Builds the room for `layout` from the shipped piece files
/// ([`crate::shipped::model`]).
pub fn room(layout: &Layout) -> Model {
    room_from(layout, |p| crate::shipped::model(p.id()).as_ref().clone())
}

/// [`room`] as the renderer draws it: per group ([`NODES`]) one primitive
/// per material, in the room's coordinates (the pieces' transforms baked
/// in, sorted by material) – the same meshes the room had before its
/// pieces were files, and a few dozen draws instead of one per piece.
pub fn room_merged(layout: &Layout) -> Model {
    merge_groups(&room(layout))
}

/// A room model with every group's pieces merged into one primitive per
/// material in the room's frame.
pub fn merge_groups(m: &Model) -> Model {
    let mut out = Model {
        materials: Vec::new(),
        copyright: m.copyright.clone(),
        generator: m.generator.clone(),
        ..Default::default()
    };
    let root = out.add(Node::new("workshop", None, Vec3::ZERO));
    for name in NODES {
        let Some(g) = m.node(name) else { continue };
        let node = out.add(Node::new(name, Some(root), Vec3::ZERO));
        let mut by: BTreeMap<String, Primitive> = BTreeMap::new();
        for (i, n) in m.nodes.iter().enumerate() {
            if n.primitives.is_empty() || !m.is_below(i, g) {
                continue;
            }
            let w = m.world(i);
            let normal = glam::Mat3::from(w.matrix3).inverse().transpose();
            for p in &n.primitives {
                let mat = &m.materials[p.material];
                let q = by.entry(mat.name.clone()).or_insert_with(|| Primitive {
                    material: out.material_or_add(mat.clone()),
                    ..Default::default()
                });
                let base = q.positions.len() as u32;
                q.positions.extend(
                    p.positions
                        .iter()
                        .map(|v| w.transform_point3(Vec3::from(*v)).to_array()),
                );
                q.normals.extend(
                    p.normals
                        .iter()
                        .map(|v| (normal * Vec3::from(*v)).normalize_or_zero().to_array()),
                );
                q.indices.extend(p.indices.iter().map(|k| k + base));
            }
        }
        out.nodes[node].primitives = by.into_values().collect();
    }
    out
}

/// Builds the room for `layout` from the pieces `piece` gives (the files,
/// or in tests the generator's [`Piece::model`]): the [`NODES`] groups
/// below a root `workshop`, each with its pieces placed (translated,
/// turned, the slabs scaled to the room).
pub fn room_from(layout: &Layout, piece: impl FnMut(Piece) -> Model) -> Model {
    let pieces: BTreeMap<Piece, Model> =
        Piece::ALL.into_iter().zip(Piece::ALL.map(piece)).collect();
    let mut m = Model {
        copyright: crate::COPYRIGHT.into(),
        generator: crate::GENERATOR.into(),
        ..Default::default()
    };
    let root = m.add(Node::new("workshop", None, Vec3::ZERO));
    let group: Vec<usize> = NODES
        .iter()
        .map(|n| m.add(Node::new(n, Some(root), Vec3::ZERO)))
        .collect();
    let g = |name: &str| group[NODES.iter().position(|n| *n == name).expect("group")];
    let (lo, hi) = layout.bounds;
    let depth = hi.z - lo.z;
    let mid_z = (lo.z + hi.z) * 0.5;
    let one = Vec3::ONE;
    let place = |m: &mut Model, grp: &str, p: Piece, at: Vec3, rot: Quat, scale: Vec3| {
        place_piece(m, g(grp), &pieces[&p], (at, rot, scale), &[]);
    };
    // A slab (a unit box piece) stretched to the box `centre` ± `half` of
    // the room, axis-aligned: scaled by the piece's measured size (so the
    // file's rounding is not stretched with it), never turned.
    let slab = |m: &mut Model, grp: &str, p: Piece, centre: Vec3, half: Vec3| {
        let src = &pieces[&p];
        let (plo, phi) = src.near_bounds().expect("a slab has a mesh");
        let scale = 2.0 * half / (phi - plo);
        let at = centre - half - plo * scale;
        place_piece(m, g(grp), src, (at, Quat::IDENTITY, scale), &[]);
    };
    // Floor: the slab, its joints in a 1.5 m grid, the mat, the pump.
    slab(
        &mut m,
        "floor",
        Piece::FloorSegment,
        Vec3::new(0.0, -0.05, mid_z),
        Vec3::new(hi.x, 0.05, depth * 0.5),
    );
    let mut x = -1.5 * (hi.x / 1.5).floor();
    while x <= hi.x {
        slab(
            &mut m,
            "floor",
            Piece::FloorJoint,
            Vec3::new(x, 0.0005, mid_z),
            Vec3::new(0.006, 0.0006, depth * 0.5),
        );
        x += 1.5;
    }
    let mut z = lo.z + 1.5;
    while z < hi.z {
        slab(
            &mut m,
            "floor",
            Piece::FloorJoint,
            Vec3::new(0.0, 0.0005, z),
            Vec3::new(hi.x, 0.0006, 0.006),
        );
        z += 1.5;
    }
    place(
        &mut m,
        "floor",
        Piece::WorkMat,
        Vec3::new(0.0, 0.0, STAND_Z),
        Quat::IDENTITY,
        one,
    );
    place(&mut m, "floor", Piece::FloorPump, PUMP, Quat::IDENTITY, one);
    // Walls: the back wall (its lower part painted), the side walls,
    // skirting, the ceiling with its lamps.
    let t = WALL_T;
    slab(
        &mut m,
        "walls",
        Piece::WallSegment,
        Vec3::new(0.0, hi.y * 0.5, lo.z - t),
        Vec3::new(hi.x + t, hi.y * 0.5 + t, t),
    );
    slab(
        &mut m,
        "walls",
        Piece::WallDado,
        Vec3::new(0.0, 0.6, lo.z + 0.002),
        Vec3::new(hi.x, 0.6, 0.002),
    );
    for side in [-1.0f32, 1.0] {
        slab(
            &mut m,
            "walls",
            Piece::WallSegment,
            Vec3::new(side * (hi.x + t), hi.y * 0.5, mid_z),
            Vec3::new(t, hi.y * 0.5 + t, depth * 0.5),
        );
        slab(
            &mut m,
            "walls",
            Piece::Skirting,
            Vec3::new(side * (hi.x - 0.008), 0.05, mid_z),
            Vec3::new(0.008, 0.05, depth * 0.5),
        );
    }
    slab(
        &mut m,
        "walls",
        Piece::Skirting,
        Vec3::new(0.0, 0.05, lo.z + 0.008),
        Vec3::new(hi.x, 0.05, 0.008),
    );
    slab(
        &mut m,
        "walls",
        Piece::CeilingSegment,
        Vec3::new(0.0, hi.y + t, mid_z),
        Vec3::new(hi.x + t, t, depth * 0.5),
    );
    for x in [-hi.x * 0.45, hi.x * 0.45, 0.0] {
        for z in [lo.z + 1.0, 0.6, 2.8] {
            if x == 0.0 && z != 0.6 {
                continue;
            }
            place(
                &mut m,
                "walls",
                Piece::CeilingLamp,
                Vec3::new(x, hi.y, z),
                Quat::IDENTITY,
                one,
            );
        }
    }
    // The stand.
    place(
        &mut m,
        "stand",
        Piece::Stand,
        layout.stand_base,
        Quat::IDENTITY,
        one,
    );
    // The hook wall: the rail along it, a hook per bike and the "+" hook.
    let plus = layout.hooks.len() - 1;
    let (first, last) = (layout.hooks[0].hook.x, layout.hooks[plus].hook.x);
    slab(
        &mut m,
        "hooks",
        Piece::HookRail,
        Vec3::new((first + last) * 0.5, HOOK_Y + 0.04, BACK_WALL_Z + 0.02),
        Vec3::new((last - first) * 0.5 + 0.5, 0.07, 0.02),
    );
    let hook = &pieces[&Piece::Hook];
    for (i, h) in layout.hooks.iter().enumerate() {
        let skip: &[&str] = if i == plus { &[] } else { &["plus_sign"] };
        place_piece(
            &mut m,
            g("hooks"),
            hook,
            (Vec3::new(h.hook.x, 0.0, BACK_WALL_Z), Quat::IDENTITY, one),
            skip,
        );
    }
    // The bench and the parts shelf, the pegboard on the left wall.
    let w = layout.half_width();
    let bench = Vec3::new(layout.bench.x, 0.0, layout.bench.z);
    place(
        &mut m,
        "bench",
        Piece::Workbench,
        bench,
        Quat::IDENTITY,
        one,
    );
    place(
        &mut m,
        "bench",
        Piece::PartsShelf,
        Vec3::new(-w, 0.0, SHELF_Z),
        Quat::IDENTITY,
        one,
    );
    place(
        &mut m,
        "pegboard",
        Piece::Pegboard,
        Vec3::new(-w, 0.0, layout.bench.z),
        Quat::IDENTITY,
        one,
    );
    // The changing corner: mirror, bench, locker, kit rail.
    let mc = layout.mirror;
    for (p, at) in [
        (Piece::Mirror, Vec3::new(mc.x, 0.0, mc.z)),
        (Piece::ChangingBench, changing_bench_at(layout)),
        (Piece::Locker, Vec3::new(w - 0.3, 0.0, LOCKER_Z)),
        (Piece::KitRail, Vec3::new(w - 0.06, 0.0, KIT_RAIL_Z)),
    ] {
        place(&mut m, "mirror_corner", p, at, Quat::IDENTITY, one);
    }
    let c = layout.window.0;
    place(
        &mut m,
        "window",
        Piece::Window,
        Vec3::new(c.x, 0.0, c.z),
        Quat::IDENTITY,
        one,
    );
    m
}

/// The locker's and the kit rail's place along the right wall (z).
const LOCKER_Z: f32 = -1.65;
const KIT_RAIL_Z: f32 = -1.05;

/// Where the changing bench stands (its floor point).
fn changing_bench_at(l: &Layout) -> Vec3 {
    Vec3::new(l.half_width() - 0.32, 0.0, l.mirror.z + 1.15)
}

/// Copies piece `src` into `m` under node `parent` at `(translation,
/// rotation, scale)`, without the nodes named in `skip` (and what is
/// below them).
fn place_piece(
    m: &mut Model,
    parent: usize,
    src: &Model,
    (translation, rotation, scale): (Vec3, Quat, Vec3),
    skip: &[&str],
) {
    let mut map: Vec<Option<usize>> = vec![None; src.nodes.len()];
    for (i, n) in src.nodes.iter().enumerate() {
        if skip.contains(&n.name.as_str()) {
            continue;
        }
        let new_parent = match n.parent {
            None => parent,
            Some(p) => match map[p] {
                Some(q) => q,
                None => continue,
            },
        };
        let mut node = Node::new(&n.name, Some(new_parent), n.translation);
        node.rotation = n.rotation;
        node.scale = n.scale;
        if n.parent.is_none() {
            let local =
                glam::Affine3A::from_scale_rotation_translation(scale, rotation, translation)
                    * n.local();
            let (s, r, t) = local.to_scale_rotation_translation();
            node.translation = t;
            node.rotation = r;
            node.scale = s;
        }
        for p in &n.primitives {
            let mut q = p.clone();
            q.material = m.material_or_add(src.materials[p.material].clone());
            node.primitives.push(q);
        }
        map[i] = Some(m.add(node));
    }
}

fn floor_pump() -> Mats {
    let mut p = Mats::default();
    // Floor pump: base, barrel, T-handle, hose.
    let pump = Vec3::ZERO;
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

/// The stand at its post's foot; `clamp` is the clamp's place from there.
fn stand(clamp: Vec3) -> Mats {
    let mut p = Mats::default();
    let base = Vec3::ZERO;
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
    let top = Vec3::new(base.x, clamp.y, base.z);
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

/// A hook's points from its floor point on the back wall: where it holds
/// the rim, and its card's centre.
fn hook_points() -> (Vec3, Vec3) {
    (
        Vec3::new(0.0, HOOK_Y, HANG_OUT),
        Vec3::new(0.0, 0.38, 0.012),
    )
}

/// A hook and its card (from its floor point on the back wall), and the
/// "+" sign of the empty hook.
fn hook_piece() -> (Mats, Mats) {
    let mut p = Mats::default();
    let (hook, card) = hook_points();
    let wall = Vec3::new(0.0, hook.y + 0.04, 0.04);
    // Plate, arm out to the rim, the coated curl that holds it.
    p.s("steel_dark")
        .cuboid(wall, Vec3::new(0.035, 0.05, 0.008), Quat::IDENTITY);
    p.s("steel").sweep(
        &[
            wall,
            hook + Vec3::new(0.0, 0.03, -0.08),
            hook + Vec3::new(0.0, 0.01, 0.0),
            hook + Vec3::new(0.0, -0.035, 0.04),
            hook + Vec3::new(0.0, -0.06, 0.035),
        ],
        &[0.007; 5],
        6,
        true,
    );
    p.s("rubber").sweep(
        &[
            hook + Vec3::new(0.0, 0.012, -0.03),
            hook + Vec3::new(0.0, -0.035, 0.04),
            hook + Vec3::new(0.0, -0.062, 0.035),
        ],
        &[0.0095; 3],
        6,
        true,
    );
    // Name card plate on the wall under the bike.
    p.s("steel_dark").cuboid(
        card - Vec3::Z * 0.002,
        Vec3::new(0.2, 0.075, 0.004),
        Quat::IDENTITY,
    );
    p.s("card").cuboid(
        card + Vec3::Z * 0.003,
        Vec3::new(0.185, 0.062, 0.003),
        Quat::IDENTITY,
    );
    // The empty hook: a "+" sign on its plate (a shape, not text).
    let mut plus = Mats::default();
    for half in [
        Vec3::new(0.045, 0.012, 0.003),
        Vec3::new(0.012, 0.045, 0.003),
    ] {
        plus.s("wall_accent")
            .cuboid(card + Vec3::Z * 0.008, half, Quat::IDENTITY);
    }
    (p, plus)
}

/// The workbench from its floor point (under the worktop's centre).
fn workbench() -> Mats {
    let mut p = Mats::default();
    let c = Vec3::new(0.0, BENCH_Y, 0.0);
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
    p
}

/// The worktop's height.
const BENCH_Y: f32 = 0.92;
/// The workbench's centre along the left wall (z).
const BENCH_Z: f32 = -1.0;

/// The parts shelf at the front of the left wall, from its floor point on
/// the wall: four boards with boxes and bins.
fn parts_shelf() -> Mats {
    let mut p = Mats::default();
    // `along` the wall as in the room (z), so the boxes fall where they
    // did; the piece's origin is at `SHELF_Z`.
    let z0 = SHELF_Z;
    let left = |out: f32, y: f32, z: f32| Vec3::new(out, y, z - SHELF_Z);
    for k in 0..2 {
        p.box_(
            "steel_dark",
            left(0.2, 0.95, z0 + if k == 0 { -0.6 } else { 0.6 }),
            Vec3::new(0.2, 0.95, 0.015),
        );
    }
    for (k, y) in [0.08f32, 0.55, 1.05, 1.55].into_iter().enumerate() {
        p.box_("wood", left(0.2, y, z0), Vec3::new(0.2, 0.015, 0.6));
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
            p.box_(mat, left(0.18, y + 0.015 + h, z + w), Vec3::new(0.15, h, w));
            z += 2.0 * w + 0.04;
            j += 1;
        }
    }
    p
}

/// The pegboard over the workbench, from its floor point on the left wall
/// (at the bench's centre).
fn pegboard() -> Mats {
    let mut p = Mats::default();
    // Along the wall as in the room (z), the piece's origin at the bench.
    let left = |out: f32, y: f32, z: f32| Vec3::new(out, y, z - BENCH_Z);
    let (z0, z1, y0, y1) = (BENCH_Z - 0.9, BENCH_Z + 0.9, 1.08, 2.25);
    let (zc, yc) = ((z0 + z1) * 0.5, (y0 + y1) * 0.5);
    let face = 0.03;
    p.box_(
        "pegboard",
        left(0.015, yc, zc),
        Vec3::new(0.015, (y1 - y0) * 0.5, (z1 - z0) * 0.5),
    );
    p.box_(
        "wood_dark",
        left(0.02, y1 + 0.02, zc),
        Vec3::new(0.02, 0.02, (z1 - z0) * 0.5 + 0.02),
    );
    p.box_(
        "wood_dark",
        left(0.02, y0 - 0.02, zc),
        Vec3::new(0.02, 0.02, (z1 - z0) * 0.5 + 0.02),
    );
    // Holes (a grid of dark dots).
    let mut y = y0 + 0.06;
    while y < y1 - 0.03 {
        let mut z = z0 + 0.06;
        while z < z1 - 0.03 {
            p.box_(
                "peg_hole",
                left(face + 0.0005, y, z),
                Vec3::new(0.0006, 0.005, 0.005),
            );
            z += 0.1;
        }
        y += 0.1;
    }
    let x = face + 0.012;
    let peg = |p: &mut Mats, y: f32, z: f32| {
        p.s("steel").tube(
            left(face, y, z),
            left(face + 0.05, y + 0.01, z),
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
            left(x, top - len * 0.5, z),
            Vec3::new(0.003, len * 0.5, 0.006 + 0.0006 * k as f32),
            Quat::IDENTITY,
        );
        p.s("steel")
            .annulus(left(x, top + r * 0.5, z), Vec3::X, r * 0.5, r, 0.006, 12);
        for dz in [-1.0f32, 1.0] {
            p.s("steel").cuboid(
                left(x, top - len - r * 0.4, z + dz * r * 0.6),
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
            left(x + 0.01, top, z),
            left(x + 0.01, top - 0.1, z),
            0.013,
            0.011,
            8,
            true,
        );
        p.s("steel").tube(
            left(x + 0.01, top - 0.1, z),
            left(x + 0.01, top - 0.22 - 0.02 * (k % 2) as f32, z),
            0.003,
            0.003,
            5,
            true,
        );
    }
    // Hammer, pliers, hex keys, tyre levers, a chain tool.
    let hz = z0 + 1.05;
    p.s("wood").tube(
        left(x + 0.01, y1 - 0.1, hz),
        left(x + 0.01, y1 - 0.42, hz),
        0.012,
        0.014,
        8,
        true,
    );
    p.s("steel").cuboid(
        left(x + 0.01, y1 - 0.09, hz),
        Vec3::new(0.015, 0.018, 0.06),
        Quat::IDENTITY,
    );
    let pz = z0 + 1.25;
    for side in [-1.0f32, 1.0] {
        p.s("tool_red").sweep(
            &[
                left(x, y1 - 0.12, pz),
                left(x, y1 - 0.2, pz + side * 0.015),
                left(x, y1 - 0.32, pz + side * 0.03),
            ],
            &[0.007, 0.008, 0.008],
            6,
            true,
        );
    }
    p.s("steel").cuboid(
        left(x, y1 - 0.1, pz),
        Vec3::new(0.005, 0.03, 0.012),
        Quat::IDENTITY,
    );
    for k in 0..7 {
        let z = z0 + 1.4 + k as f32 * 0.025;
        let len = 0.06 + 0.012 * k as f32;
        p.s("steel_dark").cuboid(
            left(x, y1 - 0.12 - len * 0.5, z),
            Vec3::new(0.003, len * 0.5, 0.003),
            Quat::IDENTITY,
        );
        p.s("steel_dark").cuboid(
            left(x + 0.015, y1 - 0.12, z),
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
            left(x, y0 + 0.3, z),
            Vec3::new(0.004, 0.06, 0.012),
            Quat::IDENTITY,
        );
    }
    peg(&mut p, y0 + 0.37, z0 + 0.285);
    // A spare tyre hung on a peg, and a cassette tool.
    p.s("rubber").torus(
        left(x + 0.02, y0 + 0.35, z0 + 0.95),
        Vec3::X,
        0.2,
        0.014,
        24,
        6,
        0.0,
    );
    peg(&mut p, y0 + 0.55, z0 + 0.95);
    p.s("steel_dark").tube(
        left(x, y0 + 0.2, z0 + 1.45),
        left(x, y0 + 0.45, z0 + 1.45),
        0.009,
        0.009,
        6,
        true,
    );
    p.s("steel").annulus(
        left(x, y0 + 0.47, z0 + 1.45),
        Vec3::X,
        0.008,
        0.02,
        0.012,
        12,
    );
    p
}

/// The mirror in its frame, from its floor point (the frame's back, at
/// the mirror's centre along the wall); it faces −X.
fn mirror() -> Mats {
    let mut p = Mats::default();
    let mc = Vec3::new(0.0, MIRROR_Y, 0.0);
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
    p
}

/// The mirror's centre height.
const MIRROR_Y: f32 = 1.25;

/// The bench in front of the mirror's right, from its floor point: a
/// pair of shoes under it and a towel on it.
fn changing_bench() -> Mats {
    let mut p = Mats::default();
    let b = Vec3::ZERO;
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
    p
}

/// The locker (two doors, handles, vents), from its floor point.
fn locker() -> Mats {
    let mut p = Mats::default();
    let lk = Vec3::new(0.0, 1.0, 0.0);
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
    p
}

/// The kit rail between locker and window with a jersey on it, from its
/// floor point.
fn kit_rail() -> Mats {
    let mut p = Mats::default();
    let rail = Vec3::new(0.0, 1.95, 0.0);
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

/// The window (centre `y` high, `size` wide along the wall and high), from
/// its floor point; a sill with a plant pot.
fn window(y: f32, size: Vec2) -> Mats {
    let mut p = Mats::default();
    let c = Vec3::new(0.0, y, 0.0);
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

    /// The room's triangles in room coordinates: (group, material,
    /// corners, corner normals).
    fn world_triangles(m: &Model) -> Vec<(String, String, [Vec3; 3], [Vec3; 3])> {
        let mut out = Vec::new();
        for (i, n) in m.nodes.iter().enumerate() {
            if n.primitives.is_empty() {
                continue;
            }
            let group = NODES
                .iter()
                .find(|g| m.node(g).is_some_and(|gi| m.is_below(i, gi)))
                .map_or_else(|| n.name.clone(), |g| g.to_string());
            let w = m.world(i);
            let normal_matrix = glam::Mat3::from(w.matrix3).inverse().transpose();
            for p in &n.primitives {
                for t in p.indices.chunks(3) {
                    let pos = [0, 1, 2]
                        .map(|k| w.transform_point3(Vec3::from(p.positions[t[k] as usize])));
                    let nor = [0, 1, 2].map(|k| {
                        (normal_matrix * Vec3::from(p.normals[t[k] as usize])).normalize()
                    });
                    out.push((
                        group.clone(),
                        m.materials[p.material].name.clone(),
                        pos,
                        nor,
                    ));
                }
            }
        }
        out
    }

    /// The room built from the generator's pieces (the files are checked
    /// against them in `the_piece_files_are_the_generators`).
    fn generated_room(l: &Layout) -> Model {
        room_from(l, Piece::model)
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
            // Every material is one of ours, and a paint slot of the same
            // colour.
            for mat in &m.materials {
                let (_, c, metal, rough) = MATERIALS
                    .iter()
                    .find(|(n, ..)| *n == mat.name)
                    .unwrap_or_else(|| panic!("{}", mat.name));
                let s = crate::slots::get(&mat.name).expect("a slot");
                assert_eq!((s.color, s.metallic, s.roughness), (*c, *metal, *rough));
            }
        }
    }

    #[test]
    fn normals_are_unit_and_agree_with_the_winding() {
        for (group, _, pos, nor) in world_triangles(&room(&layout(6))) {
            let face = (pos[1] - pos[0]).cross(pos[2] - pos[0]);
            if face.length() < 1e-12 {
                continue;
            }
            for nn in nor {
                assert!((nn.length() - 1.0).abs() < 1e-3, "{group}: {nn}");
                assert!(nn.dot(face) > -1e-9, "{group}: inverted face");
            }
        }
    }

    #[test]
    fn the_walls_face_into_the_room() {
        // The faces of the walls that bound the room look inwards.
        let l = layout(6);
        let (lo, hi) = l.bounds;
        let centre = Vec3::new(0.0, hi.y * 0.5, (lo.z + hi.z) * 0.5);
        let mut inward = 0;
        for (group, _, pos, nor) in world_triangles(&room(&l)) {
            // A face lying on the back wall's inner plane.
            if group == "walls" && (pos[0].z - lo.z).abs() < 1e-4 && nor[0].z > 0.9 {
                inward += 1;
                assert!(nor[0].dot(centre - pos[0]) > 0.0);
            }
        }
        assert!(inward >= 2, "the back wall's inner face");
    }

    /// The room from the files is the room the code generated before the
    /// pieces were files (v5.09, `workshop::room`): for 0 … 24 bikes, per
    /// group and material the same triangles, bounds, centre and area.
    #[test]
    fn the_room_matches_the_old_generated_room() {
        let reference = include_str!("../tests/data/workshop-room-generated-v5.09.txt");
        type Key = (usize, String, String);
        let mut want: BTreeMap<Key, (usize, Vec3, Vec3, Vec3, f32)> = BTreeMap::new();
        for line in reference.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let f = |k: usize| -> f32 { w[k].parse().unwrap() };
            want.insert(
                (w[0].parse().unwrap(), w[1].into(), w[2].into()),
                (
                    w[3].parse().unwrap(),
                    Vec3::new(f(4), f(5), f(6)),
                    Vec3::new(f(7), f(8), f(9)),
                    Vec3::new(f(10), f(11), f(12)),
                    f(13),
                ),
            );
        }
        let counts: Vec<usize> = want.keys().map(|k| k.0).collect();
        let mut sizes = counts.clone();
        sizes.dedup();
        assert!(sizes.len() >= 5, "{sizes:?}");
        for bikes in sizes {
            let mut got: BTreeMap<Key, Vec<Vec3>> = BTreeMap::new();
            for (group, mat, pos, _) in world_triangles(&room(&layout(bikes))) {
                got.entry((bikes, group, mat)).or_default().extend(pos);
            }
            let keys: Vec<&Key> = want.keys().filter(|k| k.0 == bikes).collect();
            assert_eq!(keys.len(), got.len(), "{bikes} bikes: groups and materials");
            for k in keys {
                let (tris, lo, hi, c, area) = want[k];
                let v = got.get(k).unwrap_or_else(|| panic!("{k:?} missing"));
                assert_eq!(v.len() / 3, tris, "{k:?}");
                let vlo = v.iter().fold(Vec3::splat(f32::MAX), |a, b| a.min(*b));
                let vhi = v.iter().fold(Vec3::splat(f32::MIN), |a, b| a.max(*b));
                let vc = v.iter().sum::<Vec3>() / v.len() as f32;
                let varea: f32 = v
                    .chunks(3)
                    .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() * 0.5)
                    .sum();
                for (a, b, what) in [(vlo, lo, "min"), (vhi, hi, "max"), (vc, c, "centre")] {
                    assert!(a.distance(b) < 2e-4, "{k:?} {what}: {a} vs {b}");
                }
                assert!(
                    (varea - area).abs() < 5e-4 * area.max(1.0),
                    "{k:?} area {varea} vs {area}"
                );
            }
        }
    }

    /// The renderer's merged room: one primitive per group and material,
    /// the same triangles in the same places as the room of pieces.
    #[test]
    fn the_merged_room_is_the_room_of_pieces() {
        for bikes in [0, 6, 24] {
            let l = layout(bikes);
            let pieces = room(&l);
            let merged = room_merged(&l);
            let sum = |m: &Model| {
                let mut out: BTreeMap<(String, String), (usize, Vec3)> = BTreeMap::new();
                for (g, mat, pos, _) in world_triangles(m) {
                    let e = out.entry((g, mat)).or_default();
                    e.0 += 1;
                    e.1 += pos[0] + pos[1] + pos[2];
                }
                out
            };
            let (a, b) = (sum(&pieces), sum(&merged));
            assert_eq!(a.len(), b.len());
            for (k, (n, s)) in &a {
                let (m, t) = b[k];
                assert_eq!(*n, m, "{k:?}");
                assert!(s.distance(t) < 1e-2, "{k:?}");
            }
            for name in NODES {
                let n = &merged.nodes[merged.node(name).unwrap()];
                let mut mats: Vec<usize> = n.primitives.iter().map(|p| p.material).collect();
                let k = mats.len();
                mats.sort();
                mats.dedup();
                assert_eq!(mats.len(), k, "{name}: one primitive per material");
            }
        }
    }

    /// Every piece is a file, the file is the generator's piece, and it
    /// keeps the contract of a workshop piece (name, budget, materials).
    #[test]
    fn the_piece_files_are_the_generators() {
        let mut files: Vec<&str> = Piece::ALL.iter().map(|p| p.file()).collect();
        files.sort();
        files.dedup();
        assert_eq!(files.len(), Piece::ALL.len());
        let (budget, _) = crate::kinds::Kind::Workshop.budget();
        for p in Piece::ALL {
            crate::kinds::check_name(p.file(), crate::kinds::Kind::Workshop).unwrap();
            let file = crate::shipped::model(p.id());
            let generated = p.model();
            assert_eq!(file.nodes.len(), generated.nodes.len(), "{p:?}");
            assert_eq!(file.nodes[0].name, p.node());
            for (a, b) in file.nodes.iter().zip(&generated.nodes) {
                assert_eq!(a.name, b.name, "{p:?}");
                assert!(a.translation.distance(b.translation) < 1e-4, "{p:?}");
                assert_eq!(a.primitives.len(), b.primitives.len(), "{p:?} {}", a.name);
                for (x, y) in a.primitives.iter().zip(&b.primitives) {
                    assert_eq!(
                        file.materials[x.material].name,
                        generated.materials[y.material].name
                    );
                    assert_eq!(x.indices, y.indices, "{p:?}");
                    for (u, v) in x.positions.iter().zip(&y.positions) {
                        assert!(
                            Vec3::from(*u).distance(Vec3::from(*v)) < 1e-4,
                            "{p:?}: {u:?} vs {v:?}"
                        );
                    }
                }
            }
            let tris = file.triangles();
            assert!(tris > 0 && tris <= budget, "{p:?}: {tris}");
        }
    }

    /// The pieces' anchors are where the layout needs them: the stand's
    /// clamp, the mirror's glass (the reflection's plane), each hook's rim
    /// point and card; the "+" only on the last hook.
    #[test]
    fn the_anchors_are_where_the_layout_puts_them() {
        for bikes in [0, 6, 24] {
            let l = layout(bikes);
            let m = room(&l);
            let at = |name: &str| -> Vec<Vec3> {
                m.nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.name == name)
                    .map(|(i, _)| Vec3::from(m.world(i).translation))
                    .collect()
            };
            assert!(at("anchor_clamp")[0].distance(l.stand_clamp) < 1e-4);
            assert!(at("anchor_glass")[0].distance(l.mirror_glass().0) < 1e-4);
            let hooks = at("anchor_hook");
            let cards = at("anchor_card");
            assert_eq!(hooks.len(), l.hooks.len());
            for (h, (a, c)) in l.hooks.iter().zip(hooks.iter().zip(&cards)) {
                assert!(a.distance(h.hook) < 1e-4, "{bikes}: {a} vs {}", h.hook);
                assert!(c.distance(h.card) < 1e-4);
            }
            let plus = at("plus_sign");
            assert_eq!(plus.len(), 1, "one + sign");
            let last = l.hooks.last().unwrap();
            assert!((plus[0].x - last.hook.x).abs() < 1e-4);
        }
        // The generator's room and the files' room are the same.
        let l = layout(6);
        let a = world_triangles(&room(&l));
        let b = world_triangles(&generated_room(&l));
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            for k in 0..3 {
                assert!(x.2[k].distance(y.2[k]) < 1e-4);
            }
        }
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

    /// The helmet's hook (4N Q28): on the mirror's wall, clear of the
    /// glass and the window by a helmet's half width, at head height, in
    /// the room, for any number of bikes.
    #[test]
    fn the_helmet_hook_is_beside_the_mirror() {
        for bikes in [1, 6, 24] {
            let l = layout(bikes);
            let h = l.helmet_hook();
            let (glass, half) = l.mirror_glass();
            assert!((h.x - glass.x).abs() < 0.1, "on the mirror's wall");
            assert!((h.z - glass.z).abs() > half.x + 0.14, "clear of the glass");
            assert!(
                (h.z - l.window.0.z).abs() > l.window.1.x * 0.5 + 0.14,
                "clear of the window"
            );
            assert!((1.3..1.8).contains(&h.y));
            assert!(h.z > l.bounds.0.z && h.z < l.bounds.1.z && h.x < l.bounds.1.x);
        }
    }
}
