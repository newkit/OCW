//! The ride photographer in the world (spec §2.11): draws what
//! [`RideView::photographer`] (the own) and [`RideView::photographers`]
//! (the other riders', spec §2.11 Q2) hold – the person (the rider model on
//! foot, plain clothes varied per photo, a camera to the eye or a drone's
//! remote, built from the avatar's parts and pose system like the riders),
//! the drone (a small quadcopter of simple shapes: body, arms, spinning
//! rotors, a camera gimbal, a blinking light) and the flash (a bright
//! point light and glow at the camera, a short white veil over the
//! rider's view when the flash is in it) – and renders the photos the app
//! asks for ([`PhotoRequests`]): an off-screen camera at the pose, at
//! 2560 × 1440, with the world camera's fog, exposure and atmosphere,
//! without the HUD, name tags or the corner stamp (UI is only drawn on
//! the window's camera), the captured image into [`PhotoCaptures`].
//!
//! The person's camera lens is exactly at the pose's eye and everything
//! of the figure is behind it (behind the photo camera's near plane), so
//! the photographer is never in the photo.

use bevy::camera::RenderTarget;
use bevy::light::NotShadowCaster;
use bevy::pbr::{AtmosphereSettings, DistanceFog};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use ocw_assets::model::Model;
use ocw_assets::pose::{self, Holding};
use ocw_core::{Rgb, RiderLook};

use crate::RideView;
use crate::camera::FollowCamera;

/// The photo's size (px) and vertical field of view (rad).
pub const PHOTO_W: u32 = 2560;
pub const PHOTO_H: u32 = 1440;
pub const PHOTO_FOV_Y: f32 = 0.70;
/// The flash's light at full brightness (lm) and reach (m).
pub const FLASH_LM: f32 = 4.0e6;
pub const FLASH_RANGE_M: f32 = 60.0;
/// The veil over the rider's view at full flash (alpha).
pub const FLASH_VEIL: f32 = 0.28;
/// The drone model's size: ~0.45 m across the rotors, seen from the road.
pub const DRONE_SCALE: f32 = 1.8;

/// The photographer (or the drone's pilot) to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PersonFigure {
    /// The feet (local metres, y up).
    pub feet: [f64; 3],
    /// The camera's lens (a photographer) or the eyes (a pilot): the
    /// figure is placed so its lens is exactly here.
    pub lens: [f64; 3],
    /// Facing (rad, clockwise from north) and the head's pitch (rad, up).
    pub yaw: f64,
    pub pitch: f64,
    pub crouch: bool,
    /// Holds the drone's remote (else a camera).
    pub remote: bool,
    /// The look (clothes, hair) of this photographer.
    pub seed: u64,
}

/// The drone to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DroneFigure {
    pub pos: [f64; 3],
    /// Its camera's view (unit).
    pub look: [f64; 3],
    /// Rotor speed 0 … 1.
    pub rotor: f32,
}

/// What [`RideView::photographer`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PhotographerView {
    pub person: Option<PersonFigure>,
    pub drone: Option<DroneFigure>,
    /// The flash: where and how bright (0 … 1).
    pub flash: Option<([f64; 3], f32)>,
}

/// A photo to render: from `eye` along `look` (local metres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoRequest {
    pub id: u64,
    pub eye: [f64; 3],
    pub look: [f64; 3],
}

/// The app's photo requests (taken in the next frames).
#[derive(Resource, Debug, Default)]
pub struct PhotoRequests(pub Vec<PhotoRequest>);

/// The rendered photos (id, image); the app takes them.
#[derive(Resource, Default)]
pub struct PhotoCaptures(pub Vec<(u64, Image)>);

pub struct PhotographerPlugin;

impl Plugin for PhotographerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhotoRequests>()
            .init_resource::<PhotoCaptures>()
            .init_resource::<Figures>()
            .init_resource::<ViewFrustum>()
            .add_systems(Startup, spawn_veil)
            .add_systems(
                PostUpdate,
                track_frustum.after(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                ((sync_person, sync_drone, sync_flash), hide_during_photo)
                    .chain()
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(Update, take_photos);
    }
}

/// The look of the photographer of `seed`: plain clothes (a long-sleeved
/// top and long trousers in everyday colours), no helmet, a varied
/// character (height, build, skin, hair, glasses) – nobody's likeness.
pub fn look_of(seed: u64) -> (RiderLook, Vec<String>) {
    const TOPS: [u32; 8] = [
        0x3B5B7A, 0x7A3B3B, 0x4F6B3A, 0xC9B48A, 0x2E2E33, 0x8C8C91, 0xB8642E, 0x5A3F6E,
    ];
    const TROUSERS: [u32; 4] = [0x2B3445, 0x3A3A3A, 0x5C4B37, 0x47524A];
    let pick =
        |n: u64, k: u64| ((seed >> (n * 5)) ^ (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> n)) % k;
    let mut look = RiderLook {
        jersey: Rgb(TOPS[pick(1, 8) as usize]),
        frame: Rgb(TROUSERS[pick(2, 4) as usize]),
        helmet: Rgb(0x30302E),
        ..RiderLook::default()
    };
    let c = &mut look.character;
    c.height_cm = 158 + pick(3, 32) as u16;
    c.build = match pick(4, 3) {
        0 => ocw_core::BodyBuild::Slim,
        1 => ocw_core::BodyBuild::Medium,
        _ => ocw_core::BodyBuild::Strong,
    };
    c.skin = pick(5, ocw_core::character::SKIN_TONES.len() as u64) as u8;
    c.hair = pick(6, ocw_core::character::HAIR_STYLES.len() as u64) as u8;
    c.face = pick(7, ocw_core::character::FACES.len() as u64) as u8;
    c.glasses = (pick(8, 3) == 0) as u8;
    c.hair_colour = ocw_core::character::HAIR_COLOURS
        [pick(9, ocw_core::character::HAIR_COLOURS.len() as u64) as usize];
    let mut kit = vec![
        "jersey-long-sleeve".to_string(),
        "shorts-tights".to_string(),
    ];
    if pick(10, 3) == 0 {
        kit.push("cap-cotton".into());
    }
    (look, kit)
}

/// The rider model's helmet nodes (never on the photographer).
const HELMETS: [&str; 4] = [
    ocw_assets::pose::nodes::HELMET_ROAD,
    ocw_assets::pose::nodes::HELMET_AERO,
    ocw_assets::rider::opt::HELMET_LIGHT,
    ocw_assets::rider::opt::HELMET_AERO_ROAD,
];

/// The spawned figures: slot 0 the own photographer, then the other
/// riders' in the order of [`RideView::photographers`].
#[derive(Resource, Default)]
struct Figures {
    slots: Vec<Slot>,
    veil: Option<Entity>,
}

#[derive(Default)]
struct Slot {
    person: Option<Person>,
    drone: Option<Drone>,
    flash: Option<Entity>,
}

/// What each slot draws this frame (slot 0: the own photographer).
fn wanted(view: Option<&RideView>) -> Vec<PhotographerView> {
    let Some(v) = view else {
        return Vec::new();
    };
    std::iter::once(v.photographer.unwrap_or_default())
        .chain(v.photographers.iter().copied())
        .collect()
}

/// Grows or shrinks the slots to `n`, despawning what goes.
fn fit_slots(commands: &mut Commands, figures: &mut Figures, n: usize) {
    while figures.slots.len() > n {
        let s = figures.slots.pop().expect("longer");
        let roots = [s.person.map(|p| p.root), s.drone.map(|d| d.root), s.flash];
        for e in roots.into_iter().flatten() {
            commands.entity(e).try_despawn();
        }
    }
    while figures.slots.len() < n {
        figures.slots.push(Slot::default());
    }
}

struct Person {
    key: (u64, bool),
    root: Entity,
    /// The model's frame (scaled by the body height).
    body: Entity,
    nodes: Vec<Entity>,
    scale: f32,
    prop: Entity,
}

struct Drone {
    root: Entity,
    rotors: [Entity; 4],
    gimbal: Entity,
    light: Entity,
    spin: f32,
}

fn mesh_of(p: &ocw_assets::model::Primitive) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.positions.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
    m.insert_indices(Indices::U32(p.indices.clone()));
    m
}

fn v3(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
}

/// The rotation that turns the model's forward (−z) to the heading `yaw`
/// (rad, clockwise from north; x east, z south).
pub fn facing(yaw: f64) -> Quat {
    Quat::from_rotation_y(-(yaw as f32))
}

#[allow(clippy::too_many_arguments)]
fn spawn_person(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    seed: u64,
    remote: bool,
) -> Person {
    let model: &Model = ocw_assets::shipped::rider_model();
    let (look, kit) = look_of(seed);
    let c = look.character.validated();
    let colours = crate::riders::avatar_colours(ocw_core::RiderId(0), false, Some(&look));
    let mut hidden: Vec<usize> = crate::riders::hidden_rider_nodes(Some(&look), &kit)
        .into_iter()
        .chain(HELMETS)
        .filter_map(|n| model.node(n))
        .collect();
    hidden.sort_unstable();
    let root = commands
        .spawn((Transform::default(), Visibility::default()))
        .id();
    let scale = c.scale();
    let body = commands
        .spawn((
            Transform::from_scale(Vec3::splat(scale)),
            Visibility::Inherited,
            ChildOf(root),
        ))
        .id();
    let mut nodes = vec![Entity::PLACEHOLDER; model.nodes.len()];
    for (i, node) in model.nodes.iter().enumerate() {
        if hidden.iter().any(|h| model.is_below(i, *h)) {
            continue;
        }
        let parent = node.parent.map_or(body, |p| nodes[p]);
        if parent == Entity::PLACEHOLDER {
            continue;
        }
        let e = commands
            .spawn((
                Transform {
                    translation: node.translation,
                    rotation: node.rotation,
                    scale: node.scale,
                },
                Visibility::Inherited,
                ChildOf(parent),
            ))
            .id();
        let girth = ocw_assets::rider::girth(model, i, c.build);
        for p in &node.primitives {
            let m = &model.materials[p.material];
            let colour = crate::riders::tint_color(&m.name, m.color, &colours);
            commands.spawn((
                Mesh3d(meshes.add(mesh_of(p))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: colour,
                    perceptual_roughness: 0.8,
                    ..default()
                })),
                Transform::from_scale(girth),
                ChildOf(e),
            ));
        }
        nodes[i] = e;
    }
    // The camera (a dark body with its lens) or the remote.
    let dark = materials.add(StandardMaterial {
        base_color: Color::srgb(0.06, 0.06, 0.07),
        perceptual_roughness: 0.4,
        ..default()
    });
    let prop = commands
        .spawn((Transform::default(), Visibility::Inherited, ChildOf(body)))
        .id();
    if remote {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.17, 0.03, 0.1))),
            MeshMaterial3d(dark.clone()),
            Transform::default(),
            ChildOf(prop),
        ));
        for x in [-0.05, 0.05] {
            commands.spawn((
                Mesh3d(meshes.add(Cylinder::new(0.006, 0.05))),
                MeshMaterial3d(dark.clone()),
                Transform::from_xyz(x, 0.035, -0.02),
                ChildOf(prop),
            ));
        }
    } else {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.13, 0.09, 0.075))),
            MeshMaterial3d(dark.clone()),
            Transform::from_xyz(0.0, 0.0, 0.035),
            ChildOf(prop),
        ));
        commands.spawn((
            Mesh3d(meshes.add(Cylinder::new(0.032, 0.07))),
            MeshMaterial3d(dark),
            Transform::from_xyz(0.0, 0.0, -0.035)
                .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            ChildOf(prop),
        ));
    }
    Person {
        key: (seed, remote),
        root,
        body,
        nodes,
        scale,
        prop,
    }
}

fn sync_person(
    mut commands: Commands,
    time: Res<Time>,
    view: Option<Res<RideView>>,
    mut figures: ResMut<Figures>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut transforms: Query<&mut Transform>,
) {
    let all = wanted(view.as_deref());
    fit_slots(&mut commands, &mut figures, all.len());
    for (i, w) in all.iter().enumerate() {
        let slot = &mut figures.slots[i];
        let Some(f) = w.person else {
            if let Some(p) = slot.person.take() {
                commands.entity(p.root).try_despawn();
            }
            continue;
        };
        let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut())
        else {
            return;
        };
        if slot
            .person
            .as_ref()
            .is_some_and(|p| p.key != (f.seed, f.remote))
        {
            let p = slot.person.take().expect("checked");
            commands.entity(p.root).try_despawn();
        }
        if slot.person.is_none() {
            slot.person = Some(spawn_person(
                &mut commands,
                meshes,
                materials,
                f.seed,
                f.remote,
            ));
        }
        let p = slot.person.as_ref().expect("spawned");
        pose_person(p, &f, time.elapsed_secs(), &mut transforms);
    }
}

/// Poses the spawned person `p` as `f` says.
fn pose_person(p: &Person, f: &PersonFigure, now_s: f32, transforms: &mut Query<&mut Transform>) {
    let rig = ocw_assets::shipped::rider_rig();
    let holding = if f.remote {
        Holding::Remote
    } else {
        Holding::Camera
    };
    let (pose, lens, hands) = pose::photographer(&rig, f.crouch, f.pitch as f32, holding, now_s);
    // The lens exactly at the pose's eye (the pilot: the eyes).
    let rot = facing(f.yaw);
    let anchor = if f.remote {
        lens - (Quat::from_rotation_x(f.pitch as f32) * Vec3::NEG_Z) * pose::LENS_AHEAD
    } else {
        lens
    };
    let at = v3(f.lens) - rot * (anchor * p.scale);
    if let Ok(mut t) = transforms.get_mut(p.root) {
        *t = Transform::from_translation(at).with_rotation(rot);
    }
    let model = ocw_assets::shipped::rider_model();
    let node = |name: &str| {
        model
            .node(name)
            .map(|i| p.nodes[i])
            .filter(|e| *e != Entity::PLACEHOLDER)
    };
    if let Some(e) = node(pose::nodes::PELVIS)
        && let Ok(mut t) = transforms.get_mut(e)
    {
        t.translation = pose.pelvis.0;
        t.rotation = pose.pelvis.1;
    }
    for (name, q) in pose.rotations() {
        if let Some(e) = node(name)
            && let Ok(mut t) = transforms.get_mut(e)
        {
            t.rotation = q;
        }
    }
    if let Ok(mut t) = transforms.get_mut(p.prop) {
        let pitch = Quat::from_rotation_x(f.pitch as f32);
        *t = if f.remote {
            Transform::from_translation(hands).with_rotation(Quat::from_rotation_x(0.5))
        } else {
            // The camera's front at the lens.
            Transform::from_translation(lens).with_rotation(pitch)
        };
    }
    let _ = p.body;
}

fn spawn_drone(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Drone {
    let body_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.82, 0.83, 0.85),
        perceptual_roughness: 0.45,
        ..default()
    });
    let dark = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.08, 0.09),
        perceptual_roughness: 0.5,
        ..default()
    });
    let blade = materials.add(StandardMaterial {
        base_color: Color::srgba(0.15, 0.15, 0.16, 0.55),
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    let light_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.1, 0.1),
        emissive: LinearRgba::rgb(40.0, 2.0, 1.5),
        ..default()
    });
    let root = commands
        .spawn((Transform::default(), Visibility::default()))
        .id();
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.16, 0.07, 0.24))),
        MeshMaterial3d(body_mat.clone()),
        Transform::default(),
        ChildOf(root),
    ));
    let mut rotors = [Entity::PLACEHOLDER; 4];
    for (i, (x, z)) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        let arm_at = Vec3::new(x * 0.11, 0.0, z * 0.11);
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.025, 0.02, 0.2))),
            MeshMaterial3d(dark.clone()),
            Transform::from_translation(arm_at * 0.55)
                .with_rotation(Quat::from_rotation_y((x * z).atan2(1.0) * 2.0)),
            ChildOf(root),
        ));
        commands.spawn((
            Mesh3d(meshes.add(Cylinder::new(0.018, 0.04))),
            MeshMaterial3d(dark.clone()),
            Transform::from_translation(arm_at + Vec3::Y * 0.02),
            ChildOf(root),
        ));
        rotors[i] = commands
            .spawn((
                Mesh3d(meshes.add(Cuboid::new(0.2, 0.004, 0.022))),
                MeshMaterial3d(blade.clone()),
                Transform::from_translation(arm_at + Vec3::Y * 0.045),
                NotShadowCaster,
                ChildOf(root),
            ))
            .id();
    }
    let gimbal = commands
        .spawn((
            Transform::from_xyz(0.0, -0.055, -0.1),
            Visibility::Inherited,
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.05, 0.045, 0.05))),
        MeshMaterial3d(dark.clone()),
        Transform::default(),
        ChildOf(gimbal),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(0.012, 0.02))),
        MeshMaterial3d(dark),
        Transform::from_xyz(0.0, 0.0, -0.03)
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        ChildOf(gimbal),
    ));
    let light = commands
        .spawn((
            Mesh3d(meshes.add(Sphere::new(0.02))),
            MeshMaterial3d(light_mat),
            Transform::from_xyz(0.0, 0.0, 0.125),
            NotShadowCaster,
            ChildOf(root),
        ))
        .id();
    Drone {
        root,
        rotors,
        gimbal,
        light,
        spin: 0.0,
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn sync_drone(
    mut commands: Commands,
    time: Res<Time>,
    view: Option<Res<RideView>>,
    mut figures: ResMut<Figures>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
) {
    let all = wanted(view.as_deref());
    fit_slots(&mut commands, &mut figures, all.len());
    for (i, w) in all.iter().enumerate() {
        let slot = &mut figures.slots[i];
        let Some(d) = w.drone else {
            if let Some(dr) = slot.drone.take() {
                commands.entity(dr.root).try_despawn();
            }
            continue;
        };
        if slot.drone.is_none() {
            let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut())
            else {
                return;
            };
            slot.drone = Some(spawn_drone(&mut commands, meshes, materials));
        }
        let dr = slot.drone.as_mut().expect("spawned");
        pose_drone(dr, &d, &time, &mut transforms, &mut visibility);
    }
}

/// Places the spawned drone `dr` as `d` says, its rotors spinning.
fn pose_drone(
    dr: &mut Drone,
    d: &DroneFigure,
    time: &Time,
    transforms: &mut Query<&mut Transform>,
    visibility: &mut Query<&mut Visibility>,
) {
    let look = v3(d.look).normalize_or(Vec3::NEG_Z);
    let yaw = look.x.atan2(-look.z);
    let pitch = look.y.clamp(-1.0, 1.0).asin();
    if let Ok(mut t) = transforms.get_mut(dr.root) {
        *t = Transform::from_translation(v3(d.pos))
            .with_rotation(Quat::from_rotation_y(-yaw))
            .with_scale(Vec3::splat(DRONE_SCALE));
    }
    if let Ok(mut t) = transforms.get_mut(dr.gimbal) {
        t.rotation = Quat::from_rotation_x(pitch);
    }
    // Rotors: up to 50 turns a second (blurred by the half-transparent
    // blades); two turn one way, two the other.
    dr.spin = (dr.spin + d.rotor * 50.0 * std::f32::consts::TAU * time.delta_secs())
        % std::f32::consts::TAU;
    for (i, r) in dr.rotors.iter().enumerate() {
        if let Ok(mut t) = transforms.get_mut(*r) {
            let dir = if i == 0 || i == 3 { 1.0 } else { -1.0 };
            t.rotation = Quat::from_rotation_y(dir * dr.spin + i as f32);
        }
    }
    // The light blinks once a second.
    if let Ok(mut v) = visibility.get_mut(dr.light) {
        let on = time.elapsed_secs().fract() < 0.15;
        v.set_if_neq(if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// The white veil over the window (the flash in the rider's view).
#[derive(Component)]
struct FlashVeil;

fn spawn_veil(mut commands: Commands, mut figures: ResMut<Figures>) {
    let e = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100.0),
                height: percent(100.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.0)),
            GlobalZIndex(40),
            Pickable::IGNORE,
            FlashVeil,
        ))
        .id();
    figures.veil = Some(e);
}

#[allow(clippy::too_many_arguments)]
fn sync_flash(
    mut commands: Commands,
    view: Option<Res<RideView>>,
    mut figures: ResMut<Figures>,
    mut lights: Query<(&mut PointLight, &mut Transform)>,
    cameras: Query<(&Camera, &GlobalTransform), With<FollowCamera>>,
    mut veil: Query<(&mut Node, &mut BackgroundColor), With<FlashVeil>>,
) {
    let all = wanted(view.as_deref());
    fit_slots(&mut commands, &mut figures, all.len());
    let mut in_view: f32 = 0.0;
    for (i, w) in all.iter().enumerate() {
        let slot = &mut figures.slots[i];
        match w.flash {
            Some((pos, f)) if f > 0.0 => {
                let at = v3(pos);
                let light = *slot.flash.get_or_insert_with(|| {
                    commands
                        .spawn((
                            PointLight {
                                intensity: 0.0,
                                range: FLASH_RANGE_M,
                                color: Color::srgb(1.0, 0.98, 0.94),
                                shadow_maps_enabled: false,
                                ..default()
                            },
                            Transform::from_translation(at),
                        ))
                        .id()
                });
                if let Ok((mut l, mut t)) = lights.get_mut(light) {
                    l.intensity = FLASH_LM * f;
                    t.translation = at;
                }
                // In the picture, or the window's camera is the flash's
                // own (the drone's view, `camera.drone pov`).
                if let Ok((cam, gt)) = cameras.single()
                    && (in_frame(cam, gt, at) || gt.translation().distance(at) < 1.0)
                {
                    in_view = in_view.max(f);
                }
            }
            _ => {
                if let Some(e) = slot.flash.take() {
                    commands.entity(e).try_despawn();
                }
            }
        }
    }
    if let Ok((mut node, mut bg)) = veil.single_mut() {
        let display = if in_view > 0.0 {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        bg.0 = Color::srgba(1.0, 1.0, 1.0, FLASH_VEIL * in_view);
    }
}

/// Whether `at` is in front of the camera and inside its picture.
pub fn in_frame(cam: &Camera, gt: &GlobalTransform, at: Vec3) -> bool {
    let ahead = (at - gt.translation()).dot(*gt.forward()) > 0.0;
    let Some(size) = cam.logical_viewport_size() else {
        return false;
    };
    ahead
        && cam
            .world_to_viewport(gt, at)
            .is_ok_and(|p| (0.0..=size.x).contains(&p.x) && (0.0..=size.y).contains(&p.y))
}

/// Which of the rider's view points are in the window's camera picture
/// (the app's photographer asks: appear and vanish out of view).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ViewFrustum {
    pub eye: Vec3,
    pub forward: Vec3,
    /// Half the horizontal and vertical field of view (rad).
    pub half_fov: Vec2,
}

impl ViewFrustum {
    /// Whether `p` is in view (a margin of 10 % around the picture).
    pub fn sees(&self, p: Vec3) -> bool {
        let d = p - self.eye;
        let f = self.forward.normalize_or(Vec3::NEG_Z);
        let ahead = d.dot(f);
        if ahead <= 0.0 || self.half_fov == Vec2::ZERO {
            return false;
        }
        let right = f.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(f);
        let x = (d.dot(right) / ahead).atan().abs();
        let y = (d.dot(up) / ahead).atan().abs();
        x <= self.half_fov.x * 1.1 && y <= self.half_fov.y * 1.1
    }
}

fn track_frustum(
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<FollowCamera>>,
    mut out: ResMut<ViewFrustum>,
) {
    let Ok((cam, gt, proj)) = cameras.single() else {
        return;
    };
    let fov = match proj {
        Projection::Perspective(p) => p.fov,
        _ => crate::camera::DEFAULT_FOV,
    };
    let aspect = cam
        .logical_viewport_size()
        .map_or(16.0 / 9.0, |s| s.x / s.y.max(1.0));
    *out = ViewFrustum {
        eye: gt.translation(),
        forward: *gt.forward(),
        half_fov: Vec2::new(((fov / 2.0).tan() * aspect).atan(), fov / 2.0),
    };
}

/// While a photo is rendered the person and the drone are hidden: they
/// are at the lens (the drone's body around it, the photographer's hands
/// and fingers beside it), so nothing of them gets into the picture; the
/// few frames coincide with the flash's veil.
fn hide_during_photo(
    figures: Res<Figures>,
    cams: Query<(), With<PhotoCamera>>,
    mut visibility: Query<&mut Visibility>,
) {
    let want = if cams.is_empty() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    // The own photographer only (the others' are other people).
    let Some(own) = figures.slots.first() else {
        return;
    };
    let roots = [
        own.person.as_ref().map(|p| p.root),
        own.drone.as_ref().map(|d| d.root),
    ];
    for e in roots.into_iter().flatten() {
        if let Ok(mut v) = visibility.get_mut(e) {
            v.set_if_neq(want);
        }
    }
}

/// A photo camera at work.
#[derive(Component)]
struct PhotoCamera {
    id: u64,
    frames: u32,
    image: Handle<Image>,
    asked: bool,
}

/// Starts the requested photos, captures them a few frames later (the
/// camera's first frames warm up its exposure and shadows), cleans up.
fn take_photos(world: &mut World) {
    let requests = std::mem::take(&mut world.resource_mut::<PhotoRequests>().0);
    if !requests.is_empty() && world.get_resource::<Assets<Image>>().is_some() {
        let follow = world
            .query_filtered::<Entity, With<FollowCamera>>()
            .iter(world)
            .next();
        for r in requests {
            let image = world
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    PHOTO_W,
                    PHOTO_H,
                    TextureFormat::Bgra8UnormSrgb,
                    None,
                ));
            let eye = v3(r.eye);
            let look = v3(r.look).normalize_or(Vec3::NEG_Z);
            let cam = world
                .spawn((
                    Camera3d::default(),
                    Camera {
                        order: -5,
                        ..default()
                    },
                    RenderTarget::Image(image.clone().into()),
                    Projection::Perspective(PerspectiveProjection {
                        fov: PHOTO_FOV_Y,
                        near: 0.05,
                        ..default()
                    }),
                    Transform::from_translation(eye).looking_to(look, Vec3::Y),
                    bevy::light::ShadowFilteringMethod::Gaussian,
                    PhotoCamera {
                        id: r.id,
                        frames: 0,
                        image,
                        asked: false,
                    },
                ))
                .id();
            // The world camera's look: fog, exposure, atmosphere, MSAA.
            if let Some(f) = follow {
                let fog = world.get::<DistanceFog>(f).cloned();
                let exposure = world.get::<bevy::camera::Exposure>(f).cloned();
                let atmosphere = world.get::<AtmosphereSettings>(f).cloned();
                let msaa = world.get::<Msaa>(f).cloned();
                let mut e = world.entity_mut(cam);
                if let Some(c) = fog {
                    e.insert(c);
                }
                if let Some(c) = exposure {
                    e.insert(c);
                }
                if let Some(c) = atmosphere {
                    e.insert(c);
                }
                if let Some(c) = msaa {
                    e.insert(c);
                }
            }
        }
    }
    let cams: Vec<(Entity, u32, bool, Handle<Image>, u64)> = world
        .query::<(Entity, &PhotoCamera)>()
        .iter(world)
        .map(|(e, p)| (e, p.frames, p.asked, p.image.clone(), p.id))
        .collect();
    for (e, frames, asked, image, id) in cams {
        if frames >= 4 && !asked {
            world.spawn(Screenshot::image(image)).observe(
                move |ev: On<ScreenshotCaptured>, mut out: ResMut<PhotoCaptures>| {
                    out.0.push((id, ev.image.clone()));
                },
            );
            if let Some(mut p) = world.get_mut::<PhotoCamera>(e) {
                p.asked = true;
            }
        }
        if frames >= 30 {
            world.entity_mut(e).despawn();
        } else if let Some(mut p) = world.get_mut::<PhotoCamera>(e) {
            p.frames += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photographers_look_different_and_wear_no_helmet() {
        let looks: Vec<_> = (0..12u64).map(|s| look_of(s * 7919 + 3)).collect();
        let tops: std::collections::BTreeSet<u32> = looks.iter().map(|(l, _)| l.jersey.0).collect();
        assert!(tops.len() >= 4, "varied clothes: {tops:?}");
        let heights: std::collections::BTreeSet<u16> =
            looks.iter().map(|(l, _)| l.character.height_cm).collect();
        assert!(heights.len() >= 4);
        for (l, kit) in &looks {
            assert!(kit.iter().any(|k| k == "jersey-long-sleeve"));
            assert!((158..190).contains(&l.character.height_cm));
        }
        // Helmets are hidden: their nodes are never drawn.
        let model = ocw_assets::shipped::rider_model();
        for h in HELMETS {
            assert!(model.node(h).is_some(), "{h} is a node of the rider model");
        }
    }

    #[test]
    fn the_figure_faces_its_heading() {
        // North (yaw 0) is −z, east (90°) is +x.
        assert!((facing(0.0) * Vec3::NEG_Z - Vec3::NEG_Z).length() < 1e-5);
        let east = facing(std::f64::consts::FRAC_PI_2) * Vec3::NEG_Z;
        assert!((east - Vec3::X).length() < 1e-5, "{east}");
    }
}
