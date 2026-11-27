//! The road vehicles drawn with their models (spec 14 Phase 18, 18A, 18B
//! *Models*, 18E T7): what [`RideView::vehicles`] holds, with the T6 files
//! of `artwork/3dmodels/vehicles/`.
//!
//! - **Instanced:** each model's meshes are merged by level of detail into
//!   one mesh (the near body, `lod1`, the box beyond; wheels and the
//!   articulated lorry's trailer apart, to turn), all drawn with one
//!   material ([`VehicleMaterial`], `vehicle.wesl`): the paint and the
//!   lamps of a vehicle come from its instance tag ([`lamps::tag`]), so
//!   every vehicle of a model and level is in one instanced draw.
//! - **Level of detail:** [`super::lod::plan`] within the quality level's
//!   budget (18A's table); shadows only from near vehicles within
//!   [`super::lod::SHADOW_M`] on Medium and High.
//! - **Lights:** the lamps glow from [`lamps::lamps`] (night, dusk, rain,
//!   fog, tunnels; brakes; indicators; a bus's windows); the near
//!   vehicles' headlights shine as faint cones in the air, the nearest
//!   few light the road (spot lights without shadows, none on Low), and
//!   far away the lamps stay dots of a few pixels.
//! - **Moving parts:** wheels turn with the distance driven; the trailer
//!   follows the kingpin's track ([`lamps::trailer_yaw`]).
//! - **Spray:** on a wet road the riders' spray plumes behind the rear
//!   wheels, bigger behind lorries and buses
//!   ([`crate::weather::spray::amount`]).

use std::collections::HashMap;

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::system::SystemParam;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshTag, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use ocw_assets::model::Model;
use ocw_sim::traffic::VehicleClass;

use super::lamps::{self, LampScene, LampState, Role};
use super::lod::{self, Budget, Lod};
use super::look::{PaletteRegion, VehicleModel, paint};
use super::{TrafficDebug, VehicleView};
use crate::RideView;
use crate::camera::FollowCamera;
use crate::weather::spray::{SprayMaterial, SprayUniform, amount, plume_mesh};

/// The vehicles' material: the standard PBR one with `vehicle.wesl`.
pub type VehicleMaterial = ExtendedMaterial<StandardMaterial, VehicleGlow>;

/// The lamps' brightness this frame (one uniform for every vehicle).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, PartialEq)]
pub struct VehicleGlow {
    /// x headlamps, y brake / tail, z indicators, w interior
    /// (display-referred emission).
    #[uniform(100)]
    pub level: Vec4,
}

impl MaterialExtension for VehicleGlow {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("vehicle.wesl")).with_source("embedded"),
        )
    }
}

/// The lamps' glow by the light of the scene: by day the lamps must be
/// brighter to show at all, at night less is plenty.
pub fn glow_levels(dark: bool) -> Vec4 {
    if dark {
        Vec4::new(9.0, 6.0, 8.0, 2.5)
    } else {
        Vec4::new(14.0, 10.0, 14.0, 0.0)
    }
}

/// The kingpin to the trailer's middle axle (m; the T6 semi-trailer).
pub const TRAILER_AXLES_M: f32 = 8.4;
/// Far lamps stay this big on the screen (px).
pub const DOT_PX: f32 = 2.0;

pub struct VehicleModelsPlugin;

impl Plugin for VehicleModelsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "vehicle.wesl");
        app.add_plugins(MaterialPlugin::<VehicleMaterial>::default());
        app.init_resource::<VehicleAssets>()
            .init_resource::<TrafficDebug>()
            .init_resource::<Drawn>()
            .init_resource::<TrafficDrawStats>()
            .add_systems(
                PostUpdate,
                sync_vehicles.before(TransformSystems::Propagate),
            );
    }
}

/// A wheel of a model: its mesh about its own pivot (the axle along x).
#[derive(Clone)]
struct WheelPart {
    mesh: Handle<Mesh>,
    pivot: Vec3,
    radius: f32,
    on_trailer: bool,
}

/// One model's meshes.
#[derive(Clone)]
struct ModelParts {
    near: Handle<Mesh>,
    lod1: Handle<Mesh>,
    boxed: Handle<Mesh>,
    wheels: Vec<WheelPart>,
    /// The trailer's mesh about the kingpin, and the kingpin.
    trailer: Option<(Handle<Mesh>, Vec3)>,
    /// Both headlights' cones in one mesh.
    cones: Handle<Mesh>,
    /// The lamps' middle at the front and the back (for the far dots and
    /// the spot lights; the back on the trailer for the articulated lorry).
    front: Vec3,
    back: Vec3,
    /// The rear wheels' contact points (spray).
    rear: Vec<Vec3>,
    /// Model z of the middle of its length: the view's position is the
    /// middle of the vehicle.
    centre_z: f32,
}

/// The vehicles' shared assets.
#[derive(Resource, Default)]
pub struct VehicleAssets {
    models: HashMap<VehicleModel, ModelParts>,
    material: Option<Handle<VehicleMaterial>>,
    cone_material: Option<Handle<StandardMaterial>>,
    dot: Option<(
        Handle<Mesh>,
        Handle<StandardMaterial>,
        Handle<StandardMaterial>,
    )>,
    spray: Option<(Handle<Mesh>, Handle<SprayMaterial>)>,
}

/// What is drawn of one vehicle.
struct Shown {
    root: Entity,
    body: Entity,
    model: VehicleModel,
    lod: Lod,
    paint: u32,
    wheels: Vec<(Entity, Vec3, f32, bool)>,
    trailer: Option<(Entity, Entity, Vec3)>,
    cones: Entity,
    dots: [Entity; 2],
    plumes: Vec<Entity>,
    travelled: f32,
    trail: Vec<[f32; 2]>,
    shadows: bool,
    tag: u32,
}

#[derive(Resource, Default)]
struct Drawn {
    shown: HashMap<u32, Shown>,
    spots: Vec<Entity>,
}

/// What was drawn last frame (tests, `traffic.draw`).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct TrafficDrawStats {
    pub near: usize,
    pub lod1: usize,
    pub boxes: usize,
    /// Vehicles in the view but not drawn (beyond the budget or range).
    pub culled: usize,
    /// Spot lights in use.
    pub spots: usize,
    /// Lamps lit: headlights, brake lamps, indicators lit now.
    pub head: usize,
    pub brake: usize,
    pub blinking: usize,
    /// Plumes of spray shown.
    pub spray: usize,
}

/// A vehicle part of the body mesh.
#[derive(Component)]
pub struct VehicleBody {
    pub id: u32,
}

/// A merged mesh under construction.
#[derive(Default)]
struct Builder {
    pos: Vec<[f32; 3]>,
    nor: Vec<[f32; 3]>,
    col: Vec<[f32; 4]>,
    uv: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

impl Builder {
    fn add(
        &mut self,
        p: &ocw_assets::model::Primitive,
        mat: &ocw_assets::model::Material,
        xf: bevy::math::Affine3A,
        shade: f32,
        paint_slot: &str,
        bus: bool,
    ) {
        let base = self.pos.len() as u32;
        let painted = mat.name == paint_slot;
        for (i, v) in p.positions.iter().enumerate() {
            let w = xf.transform_point3(Vec3::from(*v));
            self.pos.push(w.to_array());
            let n = p.normals.get(i).map_or(Vec3::Y, |n| {
                xf.transform_vector3(Vec3::from(*n)).normalize_or_zero()
            });
            self.nor.push(n.to_array());
            let vc = p
                .colors
                .get(i)
                .map_or([1.0; 4], |c| ocw_assets::model::shaded(*c, shade));
            let c = if painted {
                // The tag gives the colour; the vertex keeps the shading.
                let s = (vc[0] + vc[1] + vc[2]) / 3.0;
                let m = (mat.color[0] + mat.color[1] + mat.color[2]) / 3.0;
                let k = if m > 0.02 { (s / m).min(1.5) } else { s };
                [k, k, k, 1.0]
            } else {
                [
                    vc[0] * mat.color[0],
                    vc[1] * mat.color[1],
                    vc[2] * mat.color[2],
                    1.0,
                ]
            };
            self.col.push(c);
            let role = Role::of(&mat.name, w.x, paint_slot, bus);
            self.uv
                .push([role as u32 as f32, mat.roughness.clamp(0.04, 1.0)]);
        }
        self.idx.extend(p.indices.iter().map(|i| i + base));
    }

    fn quad(&mut self, corners: [Vec3; 4], n: Vec3, role: Role, colour: [f32; 4], rough: f32) {
        let base = self.pos.len() as u32;
        for c in corners {
            self.pos.push(c.to_array());
            self.nor.push(n.to_array());
            self.col.push(colour);
            self.uv.push([role as u32 as f32, rough]);
        }
        self.idx
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn mesh(self) -> Mesh {
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.nor);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.col);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uv);
        m.insert_indices(Indices::U32(self.idx));
        m
    }
}

/// An axis box (lo, hi) as six faces, counter-clockwise from outside.
fn add_box(b: &mut Builder, lo: Vec3, hi: Vec3, role: Role, colour: [f32; 4]) {
    let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    let (a, h) = (lo, hi);
    let faces = [
        // −Z (front), +Z (back), −X, +X, +Y, −Y.
        (
            [
                c(h.x, a.y, a.z),
                c(a.x, a.y, a.z),
                c(a.x, h.y, a.z),
                c(h.x, h.y, a.z),
            ],
            Vec3::NEG_Z,
        ),
        (
            [
                c(a.x, a.y, h.z),
                c(h.x, a.y, h.z),
                c(h.x, h.y, h.z),
                c(a.x, h.y, h.z),
            ],
            Vec3::Z,
        ),
        (
            [
                c(a.x, a.y, a.z),
                c(a.x, a.y, h.z),
                c(a.x, h.y, h.z),
                c(a.x, h.y, a.z),
            ],
            Vec3::NEG_X,
        ),
        (
            [
                c(h.x, a.y, h.z),
                c(h.x, a.y, a.z),
                c(h.x, h.y, a.z),
                c(h.x, h.y, h.z),
            ],
            Vec3::X,
        ),
        (
            [
                c(a.x, h.y, a.z),
                c(a.x, h.y, h.z),
                c(h.x, h.y, h.z),
                c(h.x, h.y, a.z),
            ],
            Vec3::Y,
        ),
        (
            [
                c(a.x, a.y, h.z),
                c(a.x, a.y, a.z),
                c(h.x, a.y, a.z),
                c(h.x, a.y, h.z),
            ],
            Vec3::NEG_Y,
        ),
    ];
    for (q, n) in faces {
        b.quad(q, n, role, colour, 0.5);
    }
}

/// The far impostor: a painted box in the model's bounds with dark
/// windows, its headlamps at the front and its lamps at the back.
fn box_mesh(lo: Vec3, hi: Vec3, class: VehicleClass) -> Mesh {
    let mut b = Builder::default();
    let wheel = 0.25f32.min((hi.y - lo.y) * 0.2);
    add_box(
        &mut b,
        Vec3::new(lo.x, lo.y + wheel, lo.z),
        hi,
        Role::Paint,
        [0.9, 0.9, 0.9, 1.0],
    );
    // A dark band of windows on top (not on lorries' boxes).
    if !matches!(class, VehicleClass::Lorry) {
        let y0 = lo.y + (hi.y - lo.y) * 0.62;
        add_box(
            &mut b,
            Vec3::new(lo.x - 0.01, y0, lo.z + (hi.z - lo.z) * 0.2),
            Vec3::new(hi.x + 0.01, hi.y - 0.08, hi.z - (hi.z - lo.z) * 0.12),
            Role::Fixed,
            [0.04, 0.05, 0.06, 1.0],
        );
    }
    // Lamps: two at each end.
    let y = lo.y + wheel + 0.35;
    let w = (hi.x - lo.x) * 0.5;
    for side in [-1.0f32, 1.0] {
        let x = side * w * 0.72;
        let q = |z: f32, d: f32| {
            [
                Vec3::new(x - 0.15 * d, y - 0.08, z),
                Vec3::new(x + 0.15 * d, y - 0.08, z),
                Vec3::new(x + 0.15 * d, y + 0.08, z),
                Vec3::new(x - 0.15 * d, y + 0.08, z),
            ]
        };
        b.quad(
            q(lo.z - 0.02, -1.0),
            Vec3::NEG_Z,
            Role::Head,
            [0.9, 0.9, 0.85, 1.0],
            0.2,
        );
        b.quad(
            q(hi.z + 0.02, 1.0),
            Vec3::Z,
            Role::Brake,
            [0.5, 0.05, 0.03, 1.0],
            0.2,
        );
    }
    b.mesh()
}

/// Cones of light ahead of the headlamps (−Z): rings from the lamp to
/// `length`, the brightness in the vertex colour fading to nothing.
fn cone_mesh(lamps: &[Vec3], length: f32, radius: f32) -> Mesh {
    let mut b = Builder::default();
    const SEG: u32 = 10;
    for &at in lamps {
        let base = b.pos.len() as u32;
        for ring in 0..3u32 {
            let t = ring as f32 / 2.0;
            let r = 0.08 + radius * t;
            let z = at.z - length * t;
            let y = at.y - 0.5 * t;
            let k = (1.0 - t).powi(2) * 0.6;
            for s in 0..SEG {
                let a = s as f32 / SEG as f32 * std::f32::consts::TAU;
                b.pos.push([at.x + r * a.cos(), y + r * 0.5 * a.sin(), z]);
                b.nor.push([0.0, 1.0, 0.0]);
                b.col.push([k, k * 0.95, k * 0.85, 1.0]);
                b.uv.push([0.0, 0.0]);
            }
        }
        for ring in 0..2u32 {
            for s in 0..SEG {
                let i0 = base + ring * SEG + s;
                let i1 = base + ring * SEG + (s + 1) % SEG;
                let j0 = i0 + SEG;
                let j1 = i1 + SEG;
                b.idx.extend([i0, j0, j1, i0, j1, i1]);
            }
        }
    }
    b.mesh()
}

fn model_parts(id: VehicleModel, model: &Model, meshes: &mut Assets<Mesh>) -> ModelParts {
    let lod = model.node("lod1");
    let trailer_node = model.node("trailer");
    let paint_slot = id.paint_slot();
    let bus = id == VehicleModel::Bus;
    let shade = model.shade_scale();
    let mut near = Builder::default();
    let mut far = Builder::default();
    let mut trailer = Builder::default();
    let mut wheels = Vec::new();
    let mut heads = Vec::new();
    let mut brakes = Vec::new();
    let kingpin = trailer_node.map(|t| Vec3::from(model.world(t).translation));
    for (i, n) in model.nodes.iter().enumerate() {
        if model.under_named(i, |s| s.starts_with("col_")) {
            continue;
        }
        let in_far = lod.is_some_and(|l| model.is_below(i, l));
        let on_trailer = trailer_node.is_some_and(|t| model.is_below(i, t));
        let world = model.world(i);
        if !in_far {
            if n.name.starts_with("light_head") {
                heads.push(Vec3::from(world.translation));
            }
            if n.name.starts_with("light_brake") {
                brakes.push(Vec3::from(world.translation));
            }
        }
        if n.primitives.is_empty() {
            continue;
        }
        if in_far {
            for p in &n.primitives {
                far.add(
                    p,
                    &model.materials[p.material],
                    world,
                    shade,
                    paint_slot,
                    bus,
                );
            }
            continue;
        }
        if n.name.starts_with("wheel_") {
            let pivot = Vec3::from(world.translation);
            let mut b = Builder::default();
            let local = bevy::math::Affine3A::from_translation(-pivot) * world;
            for p in &n.primitives {
                b.add(
                    p,
                    &model.materials[p.material],
                    local,
                    shade,
                    paint_slot,
                    bus,
                );
            }
            let radius = b
                .pos
                .iter()
                .map(|v| (v[1] * v[1] + v[2] * v[2]).sqrt())
                .fold(0.0f32, f32::max);
            let (pivot, on) = match kingpin {
                Some(k) if on_trailer => (pivot - k, true),
                _ => (pivot, false),
            };
            wheels.push(WheelPart {
                mesh: meshes.add(b.mesh()),
                pivot,
                radius: radius.max(0.1),
                on_trailer: on,
            });
            continue;
        }
        if on_trailer && let Some(k) = kingpin {
            let local = bevy::math::Affine3A::from_translation(-k) * world;
            for p in &n.primitives {
                trailer.add(
                    p,
                    &model.materials[p.material],
                    local,
                    shade,
                    paint_slot,
                    bus,
                );
            }
            continue;
        }
        for p in &n.primitives {
            near.add(
                p,
                &model.materials[p.material],
                world,
                shade,
                paint_slot,
                bus,
            );
        }
    }
    let (lo, hi) = model
        .near_bounds()
        .unwrap_or((Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 1.5, 2.0)));
    let class = match id {
        VehicleModel::RigidLorry | VehicleModel::ArticulatedLorry => VehicleClass::Lorry,
        VehicleModel::Bus => VehicleClass::Bus,
        _ => VehicleClass::Car,
    };
    let mid = |v: &[Vec3], fallback: Vec3| {
        if v.is_empty() {
            fallback
        } else {
            v.iter().copied().sum::<Vec3>() / v.len() as f32
        }
    };
    let front = mid(&heads, Vec3::new(0.0, 0.7, lo.z));
    let back = mid(&brakes, Vec3::new(0.0, 0.9, hi.z));
    // The rear wheels: the axle farthest back (on the trailer's own frame
    // for the articulated lorry: drawn straight behind).
    let mut rear: Vec<Vec3> = Vec::new();
    let back_z = wheels
        .iter()
        .map(|w| {
            w.pivot.z
                + if w.on_trailer {
                    kingpin.map_or(0.0, |k| k.z)
                } else {
                    0.0
                }
        })
        .fold(f32::MIN, f32::max);
    for w in &wheels {
        let z = w.pivot.z
            + if w.on_trailer {
                kingpin.map_or(0.0, |k| k.z)
            } else {
                0.0
            };
        if (z - back_z).abs() < 0.3 {
            let x = w.pivot.x;
            rear.push(Vec3::new(x, 0.0, z + w.radius * 0.6));
        }
    }
    let cone_len = if class == VehicleClass::Car {
        9.0
    } else {
        11.0
    };
    ModelParts {
        near: meshes.add(near.mesh()),
        lod1: if far.pos.is_empty() {
            meshes.add(box_mesh(lo, hi, class))
        } else {
            meshes.add(far.mesh())
        },
        boxed: meshes.add(box_mesh(lo, hi, class)),
        wheels,
        trailer: kingpin
            .filter(|_| !trailer.pos.is_empty())
            .map(|k| (meshes.add(trailer.mesh()), k)),
        cones: meshes.add(cone_mesh(&heads, cone_len, 2.2)),
        front,
        back,
        rear,
        centre_z: (lo.z + hi.z) * 0.5,
    }
}

fn ensure_assets(
    a: &mut VehicleAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<VehicleMaterial>,
    standard: &mut Assets<StandardMaterial>,
    spray: Option<&mut Assets<SprayMaterial>>,
) {
    if a.material.is_some() {
        return;
    }
    for m in VehicleModel::ALL {
        let model = ocw_assets::shipped::model(m.id());
        a.models.insert(m, model_parts(m, &model, meshes));
    }
    a.material = Some(materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.5,
            ..default()
        },
        extension: VehicleGlow {
            level: glow_levels(false),
        },
    }));
    a.cone_material = Some(standard.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.24, 0.22),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        double_sided: true,
        cull_mode: None,
        fog_enabled: true,
        ..default()
    }));
    let dot = |c: LinearRgba| StandardMaterial {
        base_color: c.into(),
        emissive: c,
        unlit: true,
        fog_enabled: false,
        ..default()
    };
    a.dot = Some((
        meshes.add(Sphere::new(1.0).mesh().ico(1).expect("ico sphere")),
        standard.add(dot(LinearRgba::rgb(9.0, 8.6, 7.6))),
        standard.add(dot(LinearRgba::rgb(7.0, 0.35, 0.2))),
    ));
    if let Some(s) = spray {
        a.spray = Some((
            meshes.add(plume_mesh()),
            s.add(SprayMaterial {
                spray: SprayUniform::default(),
            }),
        ));
    }
}

/// The camera's eye and its pixels per radian.
fn eye_and_focal(
    cams: &Query<(&Camera, &GlobalTransform, &Projection), With<FollowCamera>>,
) -> Option<(Vec3, f32)> {
    let (cam, t, proj) = cams.iter().next()?;
    let h = cam.logical_viewport_size().map_or(720.0, |s| s.y);
    let fov = match proj {
        Projection::Perspective(p) => p.fov,
        _ => 0.8,
    };
    Some((t.translation(), h / (2.0 * (fov / 2.0).tan())))
}

fn lod_mesh(p: &ModelParts, lod: Lod) -> Handle<Mesh> {
    match lod {
        Lod::Near => p.near.clone(),
        Lod::Lod1 => p.lod1.clone(),
        Lod::Box => p.boxed.clone(),
    }
}

fn show(mut v: Mut<Visibility>, on: bool) {
    v.set_if_neq(if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_vehicle(
    commands: &mut Commands,
    a: &VehicleAssets,
    v: &VehicleView,
    model: VehicleModel,
    lod: Lod,
    paint: u32,
    root_tf: Transform,
) -> Shown {
    let parts = &a.models[&model];
    let mat = a.material.clone().expect("vehicle material");
    let tag = MeshTag::new(paint);
    let root = commands
        .spawn((root_tf, Visibility::default(), Name::new("vehicle")))
        .id();
    let body = commands
        .spawn((
            Mesh3d(lod_mesh(parts, lod)),
            MeshMaterial3d(mat.clone()),
            tag.clone(),
            Transform::default(),
            VehicleBody { id: v.id },
            NotShadowCaster,
            ChildOf(root),
        ))
        .id();
    let trailer = parts.trailer.as_ref().map(|(mesh, k)| {
        let pivot = commands
            .spawn((
                Transform::from_translation(*k),
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        let tb = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                tag.clone(),
                Transform::default(),
                VehicleBody { id: v.id },
                NotShadowCaster,
                ChildOf(pivot),
            ))
            .id();
        (pivot, tb, *k)
    });
    let wheels = parts
        .wheels
        .iter()
        .map(|w| {
            let parent = match (&trailer, w.on_trailer) {
                (Some((p, _, _)), true) => *p,
                _ => root,
            };
            let e = commands
                .spawn((
                    Mesh3d(w.mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    tag.clone(),
                    Transform::from_translation(w.pivot),
                    Visibility::Hidden,
                    NotShadowCaster,
                    ChildOf(parent),
                ))
                .id();
            (e, w.pivot, w.radius, w.on_trailer)
        })
        .collect();
    let cones = commands
        .spawn((
            Mesh3d(parts.cones.clone()),
            MeshMaterial3d(a.cone_material.clone().expect("cone material")),
            Transform::default(),
            Visibility::Hidden,
            NotShadowCaster,
            NotShadowReceiver,
            ChildOf(root),
        ))
        .id();
    let (dot_mesh, white, red) = a.dot.clone().expect("dot");
    let dots = [(parts.front, white), (parts.back, red)].map(|(at, m)| {
        commands
            .spawn((
                Mesh3d(dot_mesh.clone()),
                MeshMaterial3d(m),
                Transform::from_translation(at),
                Visibility::Hidden,
                NotShadowCaster,
                NotShadowReceiver,
                ChildOf(root),
            ))
            .id()
    });
    let plumes = match &a.spray {
        Some((mesh, m)) => parts
            .rear
            .iter()
            .map(|at| {
                commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(m.clone()),
                        Transform::from_translation(*at),
                        Visibility::Hidden,
                        NoFrustumCulling,
                        NotShadowCaster,
                        NotShadowReceiver,
                        ChildOf(root),
                    ))
                    .id()
            })
            .collect(),
        None => Vec::new(),
    };
    Shown {
        root,
        body,
        model,
        lod,
        paint,
        wheels,
        trailer,
        cones,
        dots,
        plumes,
        travelled: 0.0,
        trail: Vec::new(),
        shadows: false,
        tag: paint,
    }
}

/// The root transform of a vehicle: on its position, facing its heading,
/// the model's middle of its length on the view's point.
fn root_transform(v: &VehicleView, centre_z: f32) -> Transform {
    let rot = Quat::from_rotation_y(v.yaw());
    let pos = crate::to_vec3(v.pos) - rot * Vec3::new(0.0, 0.0, centre_z);
    Transform::from_translation(pos).with_rotation(rot)
}

/// What the vehicles' system reads of the scene.
#[derive(SystemParam)]
struct Scene<'w> {
    view: Res<'w, RideView>,
    debug: Res<'w, TrafficDebug>,
    time: Res<'w, Time>,
    quality: Option<Res<'w, crate::quality::RenderQuality>>,
    ground: Option<Res<'w, crate::Ground>>,
    light: Option<Res<'w, crate::weather::SceneLight>>,
    fx: Option<Res<'w, crate::weather::WeatherFx>>,
}

/// The assets the vehicles' system makes and changes.
#[derive(SystemParam)]
struct Stores<'w> {
    meshes: Option<ResMut<'w, Assets<Mesh>>>,
    materials: Option<ResMut<'w, Assets<VehicleMaterial>>>,
    standard: Option<ResMut<'w, Assets<StandardMaterial>>>,
    spray: Option<ResMut<'w, Assets<SprayMaterial>>>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_vehicles(
    mut commands: Commands,
    scene_in: Scene,
    stores: Stores,
    mut assets: ResMut<VehicleAssets>,
    mut drawn: ResMut<Drawn>,
    mut stats: ResMut<TrafficDrawStats>,
    cams: Query<(&Camera, &GlobalTransform, &Projection), With<FollowCamera>>,
    mut tfs: Query<&mut Transform>,
    mut vis: Query<&mut Visibility>,
    mut mesh3d: Query<&mut Mesh3d>,
    mut tags: Query<&mut MeshTag>,
    mut spots: Query<&mut SpotLight>,
) {
    let Scene {
        view,
        debug,
        time,
        quality,
        ground,
        light,
        fx,
    } = scene_in;
    let Stores {
        meshes,
        materials,
        standard,
        spray: spray_materials,
    } = stores;
    let list: &[VehicleView] = if debug.boxes { &[] } else { &view.vehicles };
    // Gone: despawn.
    drawn.shown.retain(|id, s| {
        let keep = list.iter().any(|v| v.id == *id);
        if !keep && let Ok(mut c) = commands.get_entity(s.root) {
            c.despawn();
        }
        keep
    });
    let (Some(mut meshes), Some(mut materials), Some(mut standard)) = (meshes, materials, standard)
    else {
        return;
    };
    if list.is_empty() {
        *stats = TrafficDrawStats::default();
        for e in &drawn.spots {
            if let Ok(v) = vis.get_mut(*e) {
                show(v, false);
            }
        }
        return;
    }
    let mut spray_materials = spray_materials;
    ensure_assets(
        &mut assets,
        &mut meshes,
        &mut materials,
        &mut standard,
        spray_materials.as_deref_mut(),
    );
    let assets = &*assets;
    let eye_focal = eye_and_focal(&cams);
    let eye = eye_focal.map_or_else(|| Vec3::ZERO, |e| e.0);
    let focal = eye_focal.map_or(900.0, |e| e.1);
    let level = quality.map_or(Default::default(), |q| q.0.level);
    let budget: Budget = lod::budget(level);
    let region = view
        .position
        .map_or(PaletteRegion::default(), |(lat, lon, _)| {
            PaletteRegion::of(lat, lon)
        });
    // The scene's light for the lamps.
    let sky_light = view.sky.as_ref().map(|s| s.sky.light);
    let dark = sky_light.is_some_and(|l| l.lights_on);
    let (precip, visibility) = view.weather.as_ref().map_or((0.0, 30_000.0), |w| {
        let falling = if w.state.precipitating() {
            w.state.precip_mm_h
        } else {
            0.0
        };
        (falling, w.state.visibility_m())
    });
    let scene = LampScene {
        dark,
        precip_mm_h: precip,
        visibility_m: visibility,
    };
    if let Some(mut m) = assets.material.as_ref().and_then(|h| materials.get_mut(h)) {
        let want = glow_levels(dark);
        if m.extension.level != want {
            m.extension.level = want;
        }
    }
    let wet = view.weather.as_ref().map_or(0.0, |w| w.ground.wetness());
    let puddles = view.weather.as_ref().map_or(0.0, |w| w.ground.puddles());
    let spray_on = fx.as_ref().is_none_or(|f| f.spray);
    if let (Some(light), Some((_, h)), Some(sm)) = (
        light.as_ref(),
        assets.spray.as_ref(),
        spray_materials.as_mut(),
    ) && wet > 0.0
        && let Some(mut m) = sm.get_mut(h)
    {
        let colour = 0.6 * light.ambient_rgb * light.ambient
            + 0.2 * light.key_rgb * light.key_lux * light.towards.y.max(0.0) / std::f32::consts::PI;
        m.spray = SprayUniform {
            colour: colour.extend(0.09 + 0.08 * puddles),
            misc: Vec4::new(time.elapsed_secs() % 3600.0, 0.0, 0.0, 0.0),
        };
    }
    // Levels within the budget.
    let input: Vec<(u32, f32, Option<Lod>)> = list
        .iter()
        .map(|v| {
            (
                v.id,
                eye.distance(crate::to_vec3(v.pos)),
                drawn.shown.get(&v.id).map(|s| s.lod),
            )
        })
        .collect();
    let levels = lod::plan(&input, budget);
    let t = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    let mut st = TrafficDrawStats::default();
    // Candidates for the spot lights: (distance, root transform, front).
    let mut beams: Vec<(f32, Transform, Vec3)> = Vec::new();
    for (i, v) in list.iter().enumerate() {
        let model = VehicleModel::of(v.class, v.look);
        let Some(lod) = levels[i] else {
            st.culled += 1;
            if let Some(s) = drawn.shown.remove(&v.id)
                && let Ok(mut c) = commands.get_entity(s.root)
            {
                c.despawn();
            }
            continue;
        };
        match lod {
            Lod::Near => st.near += 1,
            Lod::Lod1 => st.lod1 += 1,
            Lod::Box => st.boxes += 1,
        }
        let dist = input[i].1;
        let parts = &assets.models[&model];
        let tf = root_transform(v, parts.centre_z);
        let colour = paint(model, v.look, region);
        // A new vehicle (or an id that came back as another model).
        if drawn.shown.get(&v.id).is_some_and(|s| s.model != model)
            && let Some(old) = drawn.shown.remove(&v.id)
            && let Ok(mut c) = commands.get_entity(old.root)
        {
            c.despawn();
        }
        let s = drawn
            .shown
            .entry(v.id)
            .or_insert_with(|| spawn_vehicle(&mut commands, assets, v, model, lod, colour, tf));
        if let Ok(mut root) = tfs.get_mut(s.root) {
            *root = tf;
        }
        // The level: the body's mesh, the wheels and trailer with it.
        if s.lod != lod {
            if let Ok(mut m) = mesh3d.get_mut(s.body) {
                m.0 = lod_mesh(parts, lod);
            }
            s.lod = lod;
        }
        let near = lod == Lod::Near;
        for (e, ..) in &s.wheels {
            if let Ok(v) = vis.get_mut(*e) {
                show(v, near);
            }
        }
        if let Some((pivot, _, _)) = s.trailer
            && let Ok(v) = vis.get_mut(pivot)
        {
            show(v, near);
        }
        let shadows = near && budget.shadows && dist <= lod::SHADOW_M;
        if shadows != s.shadows {
            let mut set = |e: Entity| {
                if let Ok(mut c) = commands.get_entity(e) {
                    if shadows {
                        c.remove::<NotShadowCaster>();
                    } else {
                        c.insert(NotShadowCaster);
                    }
                }
            };
            set(s.body);
            if let Some((_, tb, _)) = s.trailer {
                set(tb);
            }
            s.shadows = shadows;
        }
        // Wheels turn, the trailer follows.
        s.travelled += v.speed.max(0.0) as f32 * dt;
        if near {
            for (e, pivot, r, _) in &s.wheels {
                if let Ok(mut w) = tfs.get_mut(*e) {
                    // Forward (−Z) rolls the top ahead: about −X.
                    *w = Transform::from_translation(*pivot)
                        .with_rotation(Quat::from_rotation_x(-lamps::wheel_turn(s.travelled, *r)));
                }
            }
        }
        if let Some((pivot, _, k)) = s.trailer {
            let king = tf.transform_point(k);
            lamps::push_trail(&mut s.trail, [king.x, king.z], 0.5, 64);
            let yaw = lamps::trailer_yaw(&s.trail, [king.x, king.z], TRAILER_AXLES_M, v.yaw());
            if let Ok(mut p) = tfs.get_mut(pivot) {
                *p = Transform::from_translation(k)
                    .with_rotation(Quat::from_rotation_y(yaw - v.yaw()));
            }
        }
        // The lamps.
        let in_tunnel = ground
            .as_ref()
            .and_then(|g| (g.0)(v.pos[0], v.pos[2]))
            .is_some_and(|h| h > v.pos[1] + 4.0);
        let l: LampState = lamps::lamps(&scene, v, in_tunnel, t);
        st.head += usize::from(l.head);
        st.brake += usize::from(l.brake);
        st.blinking += usize::from(v.indicator != super::Indicator::Off);
        let tag = lamps::tag(colour, &l);
        if tag != s.tag || colour != s.paint {
            let mut ents = vec![s.body];
            ents.extend(s.wheels.iter().map(|w| w.0));
            if let Some((_, tb, _)) = s.trailer {
                ents.push(tb);
            }
            // Spawned this frame: the entities come after the commands, the
            // tag is set next frame.
            let mut all = true;
            for e in ents {
                if let Ok(mut m) = tags.get_mut(e) {
                    m.value = tag;
                } else {
                    all = false;
                }
            }
            if all {
                s.tag = tag;
                s.paint = colour;
            }
        }
        let lit = l.head && (dark || in_tunnel);
        if let Ok(c) = vis.get_mut(s.cones) {
            show(c, near && lit && budget.cones);
        }
        if near && lit {
            beams.push((dist, tf, parts.front));
        }
        // Far: the lamps as dots of a few pixels, the end facing us.
        let to_eye = (eye - tf.translation).normalize_or_zero();
        let ahead = tf.rotation * Vec3::NEG_Z;
        let facing = to_eye.dot(ahead);
        for (k, e) in s.dots.iter().enumerate() {
            let on = !near && l.head && if k == 0 { facing > -0.2 } else { facing < 0.2 };
            if let Ok(d) = vis.get_mut(*e) {
                show(d, on);
            }
            if on && let Ok(mut d) = tfs.get_mut(*e) {
                let at = if k == 0 { parts.front } else { parts.back };
                let r = (DOT_PX * dist / focal).max(0.3);
                *d = Transform::from_translation(at).with_scale(Vec3::splat(r));
            }
        }
        // Spray behind the rear wheels on a wet road.
        let class_k = match v.class {
            VehicleClass::Lorry | VehicleClass::Bus => 1.8,
            VehicleClass::Van | VehicleClass::Tractor => 1.3,
            VehicleClass::Motorbike => 0.7,
            VehicleClass::Car => 1.0,
        };
        let k = if spray_on && lod != Lod::Box {
            amount(wet, puddles, v.speed as f32)
        } else {
            0.0
        };
        for (p, at) in s.plumes.iter().zip(&parts.rear) {
            if let Ok(pv) = vis.get_mut(*p) {
                show(pv, k > 0.03);
            }
            if k > 0.03 {
                st.spray += 1;
                if let Ok(mut pt) = tfs.get_mut(*p) {
                    *pt = Transform::from_translation(*at)
                        .with_scale(Vec3::splat((0.4 + 0.6 * k) * class_k));
                }
            }
        }
    }
    // The nearest headlights light the road (no shadows).
    beams.sort_by(|a, b| a.0.total_cmp(&b.0));
    beams.truncate(budget.spot_lights);
    while drawn.spots.len() < beams.len() {
        let e = commands
            .spawn((
                SpotLight {
                    color: Color::srgb(1.0, 0.95, 0.85),
                    intensity: 80_000.0,
                    range: 45.0,
                    outer_angle: 0.55,
                    inner_angle: 0.3,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        drawn.spots.push(e);
    }
    for (i, e) in drawn.spots.iter().enumerate() {
        let on = i < beams.len();
        if let Ok(v) = vis.get_mut(*e) {
            show(v, on);
        }
        if on {
            let (_, tf, front) = beams[i];
            let at = tf.transform_point(front);
            let dir = tf.rotation * Vec3::new(0.0, -0.12, -1.0);
            if let Ok(mut t) = tfs.get_mut(*e) {
                *t = Transform::from_translation(at).looking_to(dir, Vec3::Y);
            }
            if let Ok(mut s) = spots.get_mut(*e)
                && s.range != 45.0
            {
                s.range = 45.0;
            }
        }
    }
    st.spots = beams.len();
    *stats = st;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::Indicator;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<VehicleMaterial>()
            .init_asset::<SprayMaterial>()
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .init_resource::<TrafficDebug>()
            .init_resource::<VehicleAssets>()
            .init_resource::<Drawn>()
            .init_resource::<TrafficDrawStats>()
            .add_systems(PostUpdate, sync_vehicles);
        app.insert_resource(TrafficDebug { boxes: false });
        app
    }

    fn vehicle(id: u32, class: VehicleClass, look: u32, z: f64) -> VehicleView {
        VehicleView {
            id,
            class,
            look,
            pos: [1.5, 0.0, z],
            heading: [0.0, -1.0],
            lateral: 1.5,
            lane: 0,
            speed: 14.0,
            braking: false,
            indicator: Indicator::Off,
        }
    }

    #[test]
    fn one_instanced_group_per_model_and_level() {
        let mut app = app();
        // 40 cars, lorries and buses from 10 m to 1.2 km of the rider (no
        // camera: the local rider's place, here the route start).
        let classes = [
            VehicleClass::Car,
            VehicleClass::Lorry,
            VehicleClass::Bus,
            VehicleClass::Van,
            VehicleClass::Motorbike,
        ];
        let vs: Vec<VehicleView> = (0..40u32)
            .map(|i| {
                vehicle(
                    i,
                    classes[i as usize % 5],
                    i * 7919,
                    -10.0 - f64::from(i) * 30.0,
                )
            })
            .collect();
        let want: std::collections::BTreeSet<(VehicleModel, Lod)> = {
            let input: Vec<(u32, f32, Option<Lod>)> = vs
                .iter()
                .map(|v| (v.id, (v.pos[2].abs().hypot(1.5)) as f32, None))
                .collect();
            let lv = lod::plan(&input, lod::budget(Default::default()));
            vs.iter()
                .zip(lv)
                .filter_map(|(v, l)| Some((VehicleModel::of(v.class, v.look), l?)))
                .collect()
        };
        app.world_mut().resource_mut::<RideView>().vehicles = vs;
        app.update();
        app.update();
        // Every vehicle body of a model and level shares one mesh and the
        // one material: Bevy batches them into one instanced draw.
        let mut groups = std::collections::BTreeSet::new();
        let mut materials = std::collections::BTreeSet::new();
        let w = app.world_mut();
        let mut q = w.query::<(
            &VehicleBody,
            &Mesh3d,
            &MeshMaterial3d<VehicleMaterial>,
            &ChildOf,
        )>();
        let bodies: Vec<_> = q
            .iter(w)
            .map(|(b, m, mat, p)| (b.id, m.0.id(), mat.0.id(), p.parent()))
            .collect();
        for (_, mesh, mat, _) in &bodies {
            groups.insert(*mesh);
            materials.insert(*mat);
        }
        assert_eq!(materials.len(), 1);
        // The trailer bodies of the articulated lorries are one more group.
        let artics = usize::from(
            want.iter()
                .any(|(m, _)| *m == VehicleModel::ArticulatedLorry),
        );
        assert_eq!(groups.len(), want.len() + artics, "{want:?}");
        let stats = w.resource::<TrafficDrawStats>().clone();
        assert_eq!(stats.near + stats.lod1 + stats.boxes + stats.culled, 40);
        assert!(
            stats.near > 0 && stats.lod1 > 0 && stats.boxes > 0,
            "{stats:?}"
        );
        // The same seed gives the same colour in the tag.
        let mut tq = w.query::<(&VehicleBody, &MeshTag)>();
        for (b, t) in tq.iter(w) {
            let v = w.resource::<RideView>().vehicles[b.id as usize];
            let m = VehicleModel::of(v.class, v.look);
            assert_eq!(
                t.value & 0xFF_FFFF,
                paint(m, v.look, PaletteRegion::Central)
            );
        }
    }

    #[test]
    fn lamps_light_up_in_the_tag_and_vehicles_go() {
        let mut app = app();
        let mut v = vehicle(5, VehicleClass::Car, 42, -20.0);
        v.braking = true;
        v.indicator = Indicator::Right;
        app.world_mut().resource_mut::<RideView>().vehicles = vec![v];
        app.update();
        app.update();
        let w = app.world_mut();
        let tag = w
            .query::<(&VehicleBody, &MeshTag)>()
            .iter(w)
            .next()
            .unwrap()
            .1
            .value;
        assert!(tag & lamps::TAG_BRAKE != 0);
        assert!(tag & lamps::TAG_DRL != 0, "daylight: running lights");
        assert_eq!(w.resource::<TrafficDrawStats>().blinking, 1);
        // Gone from the view: despawned.
        w.resource_mut::<RideView>().vehicles.clear();
        app.update();
        let w = app.world_mut();
        assert_eq!(w.query::<&VehicleBody>().iter(w).count(), 0);
    }

    #[test]
    fn every_model_has_its_parts() {
        let mut meshes = Assets::<Mesh>::default();
        for m in VehicleModel::ALL {
            let model = ocw_assets::shipped::model(m.id());
            let p = model_parts(m, &model, &mut meshes);
            let wheels = p.wheels.len();
            let want = match m {
                VehicleModel::Motorbike => 2,
                VehicleModel::ArticulatedLorry => 10,
                _ => 4,
            };
            assert!(wheels >= want, "{m:?}: {wheels} wheels");
            assert!(
                p.wheels.iter().all(|w| w.radius > 0.2 && w.radius < 1.2),
                "{m:?}"
            );
            assert_eq!(
                p.trailer.is_some(),
                m == VehicleModel::ArticulatedLorry,
                "{m:?}"
            );
            assert!(!p.rear.is_empty(), "{m:?}");
            // Headlamps at the front (−Z), lamps at the back.
            assert!(p.front.z < p.back.z, "{m:?}");
            for h in [&p.near, &p.lod1, &p.boxed] {
                let mesh = meshes.get(h).unwrap();
                assert!(mesh.count_vertices() > 0, "{m:?}");
                assert!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
            }
        }
    }
}
