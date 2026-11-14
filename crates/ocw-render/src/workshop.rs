//! The workshop scene (spec §14 Phase 4, 4A–4B): a generated room on its
//! own render layer with its own camera, the rider's bikes on the repair
//! stand and the wall hooks, part hotspots, the part picker, the ride
//! window. It draws only what the app puts into [`WorkshopView`] (the
//! scene model is Bevy-free in the app) and reports pointer events on its
//! panels as [`WorkshopPointer`] events, as the menu does.
//!
//! While the workshop is open the world's camera renders the ride into a
//! small image at a quarter of the window's size and 10 fps (the ride
//! window, off with `workshop.ride_window off`: then not at all); the
//! ride's own overlays are hidden and the workshop camera is the default
//! UI camera. Everything is spawned on opening and despawned on leaving.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::picking::events::{PointerClick, PointerOver};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::text::FontSize;
use bevy::window::{PrimaryWindow, WindowRef};
use ocw_assets::model::{Model, Primitive};
use ocw_assets::pose::bike_nodes;
use ocw_assets::slots::{self, Tint};
use ocw_assets::workshop::Layout;
use ocw_core::{ProfileId, Rgb};

use crate::camera::FollowCamera;

/// The workshop's render layer (the world is on layer 0).
pub const LAYER: usize = 7;
/// Ride window: this share of the window width, at this frame rate.
pub const RIDE_WINDOW_VW: f32 = 20.0;
pub const RIDE_WINDOW_FPS: f32 = 10.0;

/// One bike in the garage as shown.
#[derive(Debug, Clone, PartialEq)]
pub struct BikeView {
    pub profile: ProfileId,
    /// The part models on it (`ocw_assets::bikes` variant ids; "stock" =
    /// the frame's own part).
    pub variants: Vec<String>,
    /// Frame paint (`None`: the rider's look's frame colour).
    pub paint: Option<Rgb>,
    /// The name card's lines (name, type and profile, weight, value).
    pub card: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HotspotView {
    /// World position (metres, the workshop's coordinates).
    pub pos: Vec3,
    pub label: String,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PickerRowView {
    /// A group header shown above this row ("Equipped", "Owned", "Shop").
    pub group: Option<String>,
    pub left: String,
    pub right: String,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DialogView {
    pub title: String,
    pub text: String,
    pub buttons: Vec<String>,
    pub focus: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PickerView {
    pub title: String,
    pub rows: Vec<PickerRowView>,
    pub focus: usize,
    pub detail: Vec<String>,
    /// Both cards side by side when a part is pinned (A) for comparing.
    pub compare: Option<Vec<String>>,
    pub dialog: Option<DialogView>,
}

/// What the workshop shows; written by the app's scene model.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct WorkshopView {
    pub open: bool,
    pub eye: Vec3,
    pub target: Vec3,
    /// Vertical field of view (rad).
    pub fov_y: f32,
    /// Hook `i` holds bike `i`; the stand's bike is shown on the stand.
    pub bikes: Vec<BikeView>,
    pub stand: usize,
    /// A swap: (going back to its hook, coming to the stand, progress 0…1).
    pub swap: Option<(usize, usize, f32)>,
    pub focus: Option<usize>,
    pub plus_focused: bool,
    pub hotspots: Vec<HotspotView>,
    pub picker: Option<PickerView>,
    /// The status strip (power, speed, trainer) and the balance.
    pub status: String,
    pub hearts: String,
    pub hint: String,
    pub caption: Option<String>,
    pub ride_window: bool,
    /// Touch mode: the ‹ Back button (spec §3.2.2).
    pub touch: bool,
    /// The rider's look's frame colour (bikes without paint).
    pub frame: Rgb,
}

impl Default for WorkshopView {
    fn default() -> Self {
        Self {
            open: false,
            eye: Vec3::new(0.0, 1.7, 4.0),
            target: Vec3::new(0.0, 1.2, 0.0),
            fov_y: 0.75,
            bikes: Vec::new(),
            stand: 0,
            swap: None,
            focus: None,
            plus_focused: false,
            hotspots: Vec::new(),
            picker: None,
            status: String::new(),
            hearts: String::new(),
            hint: String::new(),
            caption: None,
            ride_window: true,
            touch: false,
            frame: ocw_core::RiderLook::default().frame,
        }
    }
}

/// Pointer events on the workshop's panels since the app last read them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkshopPointerEvent {
    Row(usize),
    RowClick(usize),
    Button(usize),
    Back,
}

#[derive(Resource, Debug, Default)]
pub struct WorkshopPointer(pub Vec<WorkshopPointerEvent>);

/// A built bike: the model with the parts, its hotspot anchors (bike
/// coordinates) and its box (near model only).
#[derive(Debug)]
pub struct BuiltBike {
    pub model: Model,
    pub anchors: Vec<(String, Vec3)>,
    pub bounds: (Vec3, Vec3),
    pub wheel_radius: f32,
    pub wheelbase: f32,
}

/// The key of a built bike.
pub type BikeKey = (ProfileId, Vec<String>);

/// Built bikes, shared by the app (anchors, boxes) and the renderer
/// (meshes): each build once.
#[derive(Resource, Clone, Default)]
pub struct BikeModels(Arc<Mutex<HashMap<BikeKey, Arc<BuiltBike>>>>);

impl BikeModels {
    pub fn get(&self, profile: ProfileId, variants: &[String]) -> Arc<BuiltBike> {
        let mut v: Vec<String> = variants.to_vec();
        v.sort();
        v.dedup();
        let key = (profile, v);
        let mut map = self.0.lock().expect("not poisoned");
        map.entry(key.clone())
            .or_insert_with(|| Arc::new(build_bike(profile, &key.1)))
            .clone()
    }
}

fn build_bike(profile: ProfileId, variants: &[String]) -> BuiltBike {
    let rider = ocw_assets::shipped::rider_model();
    static LOW: std::sync::OnceLock<Model> = std::sync::OnceLock::new();
    let low = LOW.get_or_init(|| ocw_assets::rider::build(ocw_assets::rider::Detail::Low));
    let ids: Vec<&str> = variants.iter().map(String::as_str).collect();
    let model = ocw_assets::bikes::build_with(profile, rider, low, &ids);
    let anchors = ocw_assets::bikes::HOTSPOT_SLOTS
        .iter()
        .filter_map(|s| {
            let i = model.node(&ocw_assets::bikes::hotspot_node(s))?;
            Some((s.to_string(), model.world(i).transform_point3(Vec3::ZERO)))
        })
        .collect();
    let far = model.node(bike_nodes::LOD1);
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for (i, n) in model.nodes.iter().enumerate() {
        if far.is_some_and(|f| model.is_below(i, f)) {
            continue;
        }
        let w = model.world(i);
        for p in &n.primitives {
            for v in &p.positions {
                let v = w.transform_point3(Vec3::from(*v));
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    let rig = ocw_assets::pose::BikeRig::from_model(&model).ok();
    BuiltBike {
        anchors,
        bounds: (lo, hi),
        wheel_radius: rig.map_or(0.34, |r| r.wheel_radius),
        wheelbase: rig.map_or(1.0, |r| r.wheelbase),
        model,
    }
}

/// Where bike `i` is: on the stand, on its hook, or on its way during a
/// swap (an arc up and over, 1.2 s).
pub fn bike_pose(
    layout: &Layout,
    i: usize,
    stand: usize,
    swap: Option<(usize, usize, f32)>,
    bike: &BuiltBike,
) -> (Vec3, Quat) {
    let hook = |i: usize| {
        layout.hooks.get(i).map_or(layout.stand_bike, |h| {
            h.bike_pose(bike.wheel_radius, bike.wheelbase)
        })
    };
    let on_stand = layout.stand_bike;
    let between = |a: (Vec3, Quat), b: (Vec3, Quat), t: f32| {
        let e = t * t * (3.0 - 2.0 * t);
        let lift = (std::f32::consts::PI * e).sin() * 0.35;
        (a.0.lerp(b.0, e) + Vec3::Y * lift, a.1.slerp(b.1, e))
    };
    match swap {
        Some((from, _, t)) if from == i => between(on_stand, hook(i), t),
        Some((_, to, t)) if to == i => between(hook(i), on_stand, t),
        _ if i == stand => on_stand,
        _ => hook(i),
    }
}

/// The world corners of bike `i`'s box where it is now.
pub fn bike_corners(pose: (Vec3, Quat), bike: &BuiltBike) -> [Vec3; 8] {
    let (lo, hi) = bike.bounds;
    std::array::from_fn(|k| {
        let p = Vec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        );
        pose.0 + pose.1 * p
    })
}

pub struct WorkshopCorePlugin;

impl Plugin for WorkshopCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorkshopView>()
            .init_resource::<WorkshopPointer>()
            .init_resource::<BikeModels>();
    }
}

/// Draws the workshop.
pub struct WorkshopPlugin;

impl Plugin for WorkshopPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<WorkshopCorePlugin>() {
            app.add_plugins(WorkshopCorePlugin);
        }
        app.init_resource::<Scene>()
            .add_systems(Update, (open_close, sync_bikes, draw_ui).chain())
            .add_systems(PostUpdate, (place_camera, ride_window));
    }
}

/// The open scene's entities.
#[derive(Resource, Default)]
struct Scene {
    open: bool,
    camera: Option<Entity>,
    root: Option<Entity>,
    ui: Option<Entity>,
    image: Option<Handle<Image>>,
    /// UI roots of the ride hidden while open, with their visibility,
    /// and the HUD switch (the HUD sets its panels' visibility itself).
    hidden: Vec<(Entity, Visibility)>,
    hud_was: Option<bool>,
    /// Spawned bikes: (key, paint, entity).
    bikes: Vec<Option<(BikeKey, Rgb, Entity)>>,
    materials: HashMap<(String, [u8; 4]), Handle<StandardMaterial>>,
    meshes: HashMap<usize, NodeMeshes>,
    since_frame: f32,
}

/// A model's meshes on the GPU: `(mesh, material index)` per node.
type NodeMeshes = Vec<Vec<(Handle<Mesh>, usize)>>;
/// The ride's UI roots (hidden while the workshop is open).
type RideUiRoots<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static Visibility),
    (With<Node>, Without<ChildOf>, Without<WorkshopUi>),
>;

#[derive(Component)]
struct WorkshopUi;
#[derive(Component)]
struct StatusText;
#[derive(Component)]
struct HeartsText;
#[derive(Component)]
struct HintText;
#[derive(Component)]
struct CaptionText;
#[derive(Component)]
struct CaptionSpan;
#[derive(Component)]
struct RideWindow;
#[derive(Component)]
struct Cards;
#[derive(Component)]
struct Markers;
#[derive(Component)]
struct Panel;
#[derive(Component)]
struct DialogBox;
#[derive(Component)]
struct BackButton;

fn mesh_of(p: &Primitive) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.positions.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
    m.insert_indices(Indices::U32(p.indices.clone()));
    m
}

fn layer() -> RenderLayers {
    RenderLayers::layer(LAYER)
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn open_close(
    mut commands: Commands,
    view: Res<WorkshopView>,
    mut scene: ResMut<Scene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    roots: RideUiRoots,
    mut world_cams: Query<(Entity, &mut Camera, &mut RenderTarget), With<FollowCamera>>,
    mut hud: Option<ResMut<crate::hud::ShowHud>>,
) {
    if view.open == scene.open {
        return;
    }
    scene.open = view.open;
    if let Some(h) = hud.as_mut() {
        if view.open {
            scene.hud_was = Some(h.0);
            h.0 = false;
        } else if let Some(was) = scene.hud_was.take() {
            h.0 = was;
        }
    }
    if !view.open {
        for e in [scene.camera.take(), scene.root.take(), scene.ui.take()]
            .into_iter()
            .flatten()
        {
            commands.entity(e).despawn();
        }
        for (e, v) in scene.hidden.drain(..) {
            if let Ok(mut c) = commands.get_entity(e) {
                c.insert(v);
            }
        }
        for (_, mut cam, mut target) in &mut world_cams {
            *target = RenderTarget::Window(WindowRef::Primary);
            cam.is_active = true;
        }
        scene.bikes.clear();
        return;
    }
    // The ride window: the world camera into a small image.
    let (w, h) = windows
        .iter()
        .next()
        .map_or((1280, 720), |w| (w.physical_width(), w.physical_height()));
    let image = images.add(Image::new_target_texture(
        (w / 4).max(64),
        (h / 4).max(36),
        TextureFormat::Bgra8UnormSrgb,
        None,
    ));
    scene.image = Some(image.clone());
    for (_, mut cam, mut target) in &mut world_cams {
        *target = RenderTarget::Image(image.clone().into());
        cam.is_active = view.ride_window;
    }
    // The ride's overlays hide.
    scene.hidden = roots.iter().map(|(e, v)| (e, *v)).collect();
    for (e, _) in &scene.hidden {
        commands.entity(*e).insert(Visibility::Hidden);
    }
    let layout = ocw_assets::workshop::layout(view.bikes.len());
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: 1,
                clear_color: ClearColorConfig::Custom(Color::srgb(0.12, 0.11, 0.10)),
                ..default()
            },
            Projection::Perspective(PerspectiveProjection {
                fov: view.fov_y,
                ..default()
            }),
            Transform::from_translation(view.eye).looking_at(view.target, Vec3::Y),
            AmbientLight {
                color: Color::srgb(1.0, 0.93, 0.85),
                brightness: 260.0,
                ..default()
            },
            bevy::camera::Exposure { ev100: 7.5 },
            bevy::ui::IsDefaultUiCamera,
            layer(),
        ))
        .id();
    scene.camera = Some(camera);
    // The room.
    let room = ocw_assets::workshop::room(&layout);
    let root = commands
        .spawn((Transform::default(), Visibility::default(), layer()))
        .id();
    let mut mats: Vec<Handle<StandardMaterial>> = Vec::new();
    for m in &room.materials {
        let c = Color::linear_rgba(m.color[0], m.color[1], m.color[2], m.color[3]);
        let glow = matches!(m.name.as_str(), "lamp" | "window_glass");
        mats.push(materials.add(StandardMaterial {
            base_color: c,
            metallic: m.metallic,
            perceptual_roughness: m.roughness,
            emissive: if glow {
                LinearRgba::from(c) * 6.0
            } else {
                LinearRgba::BLACK
            },
            ..default()
        }));
    }
    let mut nodes: Vec<Entity> = Vec::new();
    for n in &room.nodes {
        let parent = n.parent.map_or(root, |p| nodes[p]);
        let e = commands
            .spawn((
                Transform {
                    translation: n.translation,
                    rotation: n.rotation,
                    scale: n.scale,
                },
                Visibility::default(),
                ChildOf(parent),
                layer(),
            ))
            .id();
        for p in &n.primitives {
            commands.spawn((
                Mesh3d(meshes.add(mesh_of(p))),
                MeshMaterial3d(mats[p.material].clone()),
                ChildOf(e),
                layer(),
            ));
        }
        nodes.push(e);
    }
    // Warm interior light: one shadow-casting spot over the stand, fills.
    commands.spawn((
        SpotLight {
            intensity: 900_000.0,
            range: 12.0,
            outer_angle: 0.9,
            inner_angle: 0.55,
            shadow_maps_enabled: true,
            color: Color::srgb(1.0, 0.9, 0.78),
            ..default()
        },
        Transform::from_translation(layout.spot).looking_at(layout.stand_bike.0, Vec3::Z),
        ChildOf(root),
        layer(),
    ));
    for f in &layout.fills {
        commands.spawn((
            PointLight {
                intensity: 160_000.0,
                range: 14.0,
                color: Color::srgb(1.0, 0.88, 0.74),
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(*f),
            ChildOf(root),
            layer(),
        ));
    }
    scene.root = Some(root);
    scene.bikes = vec![None; view.bikes.len()];
    scene.ui = Some(spawn_ui(&mut commands, image));
}

fn spawn_ui(commands: &mut Commands, image: Handle<Image>) -> Entity {
    let font = |vh: f32| TextFont {
        font_size: FontSize::Vh(vh),
        ..default()
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            GlobalZIndex(50),
            WorkshopUi,
        ))
        .with_children(|root| {
            // Bike cards and hotspot markers, placed every frame.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                Cards,
            ));
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                Markers,
            ));
            // The status strip.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: vh(1.5),
                    left: vw(2.0),
                    right: vw(2.0),
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    column_gap: vh(3.0),
                    padding: UiRect::axes(vh(1.5), vh(0.8)),
                    border_radius: BorderRadius::all(vh(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.7)),
            ))
            .with_children(|s| {
                s.spawn((
                    Text::new("Workshop"),
                    font(2.4),
                    TextColor(Color::WHITE),
                    StatusText,
                ));
                s.spawn((
                    Text::new(""),
                    font(3.6),
                    TextColor(Color::srgb(1.0, 0.45, 0.55)),
                    HeartsText,
                    TextLayout::no_wrap(),
                ));
            });
            // The ride window.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: vh(8.5),
                    right: vw(2.0),
                    width: vw(RIDE_WINDOW_VW),
                    aspect_ratio: Some(16.0 / 9.0),
                    border: UiRect::all(vh(0.3)),
                    border_radius: BorderRadius::all(vh(0.8)),
                    ..default()
                },
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.5)),
                ImageNode::new(image),
                RideWindow,
            ));
            // Caption (bench, mirror, messages).
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    bottom: vh(9.0),
                    left: vw(4.0),
                    max_width: vw(55.0),
                    padding: UiRect::axes(vh(1.6), vh(1.0)),
                    border_radius: BorderRadius::all(vh(1.0)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.8)),
                children![(
                    Text::new(""),
                    font(3.0),
                    TextColor(Color::WHITE),
                    CaptionSpan
                )],
                CaptionText,
            ));
            // The picker panel on the right.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: vh(8.5),
                    bottom: vh(8.0),
                    right: vw(1.5),
                    width: vw(40.0),
                    padding: UiRect::axes(vh(2.0), vh(1.6)),
                    flex_direction: FlexDirection::Column,
                    row_gap: vh(0.6),
                    border: UiRect::all(vh(0.3)),
                    border_radius: BorderRadius::all(vh(1.4)),
                    display: Display::None,
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.9)),
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.25)),
                Panel,
            ));
            // The dialog.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                GlobalZIndex(60),
                DialogBox,
            ));
            // The hint bar.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    bottom: vh(1.5),
                    left: vw(2.0),
                    right: vw(2.0),
                    justify_content: JustifyContent::Center,
                    padding: UiRect::axes(vh(1.5), vh(0.7)),
                    border_radius: BorderRadius::all(vh(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.7)),
                children![(
                    Text::new(""),
                    font(3.0),
                    TextColor(Color::srgb(0.85, 0.87, 0.9)),
                    HintText
                )],
            ));
            // Touch mode: the Back button (top left).
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: vh(8.5),
                    left: vw(2.0),
                    min_width: px(96),
                    min_height: px(56),
                    padding: UiRect::axes(vh(2.0), vh(1.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(vh(1.0)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.85)),
                children![(Text::new("‹ Back"), font(3.4), TextColor(Color::WHITE))],
                BackButton,
            ))
            .observe(
                |click: On<PointerClick>, mut pointer: ResMut<WorkshopPointer>| {
                    if click.button == PointerButton::Primary {
                        pointer.0.push(WorkshopPointerEvent::Back);
                    }
                },
            );
        })
        .id()
}

/// A built bike's meshes on the GPU, per node.
fn upload(model: &Model, meshes: &mut Assets<Mesh>) -> NodeMeshes {
    model
        .nodes
        .iter()
        .map(|n| {
            n.primitives
                .iter()
                .map(|p| (meshes.add(mesh_of(p)), p.material))
                .collect()
        })
        .collect()
}

fn bike_material(
    cache: &mut HashMap<(String, [u8; 4]), Handle<StandardMaterial>>,
    materials: &mut Assets<StandardMaterial>,
    slot: &str,
    model_color: [f32; 4],
    paint: Rgb,
) -> Handle<StandardMaterial> {
    let s = slots::get(slot).unwrap_or(&slots::ALL[0]);
    let color = match s.tint {
        Tint::Frame => {
            let [r, g, b] = paint.channels();
            Color::srgb_u8(r, g, b)
        }
        _ => Color::linear_rgba(
            model_color[0],
            model_color[1],
            model_color[2],
            model_color[3],
        ),
    };
    let glow = slot.starts_with("light_");
    cache
        .entry((slot.to_string(), color.to_srgba().to_u8_array()))
        .or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: color,
                metallic: s.metallic,
                perceptual_roughness: s.roughness,
                emissive: if glow {
                    LinearRgba::from(color) * 8.0
                } else {
                    LinearRgba::BLACK
                },
                ..default()
            })
        })
        .clone()
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn sync_bikes(
    mut commands: Commands,
    view: Res<WorkshopView>,
    models: Res<BikeModels>,
    mut scene: ResMut<Scene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut transforms: Query<&mut Transform>,
) {
    if !view.open || scene.root.is_none() {
        return;
    }
    let layout = ocw_assets::workshop::layout(view.bikes.len());
    let root = scene.root.expect("open");
    scene.bikes.resize(view.bikes.len(), None);
    for (i, b) in view.bikes.iter().enumerate() {
        let built = models.get(b.profile, &b.variants);
        let mut key_variants = b.variants.clone();
        key_variants.sort();
        key_variants.dedup();
        let key = (b.profile, key_variants);
        let paint = b.paint.unwrap_or(view.frame);
        let pose = bike_pose(&layout, i, view.stand, view.swap, &built);
        let current = scene.bikes[i].as_ref();
        let fresh = current.is_none_or(|(k, p, _)| *k != key || *p != paint);
        if fresh {
            if let Some((_, _, e)) = scene.bikes[i].take() {
                commands.entity(e).despawn();
            }
            let model = &built.model;
            let ptr = Arc::as_ptr(&built) as usize;
            let handles = scene
                .meshes
                .entry(ptr)
                .or_insert_with(|| upload(model, &mut meshes))
                .clone();
            let bike = commands
                .spawn((
                    Transform::from_translation(pose.0).with_rotation(pose.1),
                    Visibility::default(),
                    ChildOf(root),
                    layer(),
                ))
                .id();
            let far = model.node(bike_nodes::LOD1);
            let mut ents: Vec<Entity> = Vec::new();
            for (n, node) in model.nodes.iter().enumerate() {
                let parent = node.parent.map_or(bike, |p| ents[p]);
                let e = commands
                    .spawn((
                        Transform {
                            translation: node.translation,
                            rotation: node.rotation,
                            scale: node.scale,
                        },
                        if far.is_some_and(|f| model.is_below(n, f)) {
                            Visibility::Hidden
                        } else {
                            Visibility::Inherited
                        },
                        ChildOf(parent),
                        layer(),
                    ))
                    .id();
                for (mesh, mat) in &handles[n] {
                    let m = &model.materials[*mat];
                    let material = bike_material(
                        &mut scene.materials,
                        &mut materials,
                        &m.name,
                        m.color,
                        paint,
                    );
                    commands.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material),
                        ChildOf(e),
                        layer(),
                    ));
                }
                ents.push(e);
            }
            scene.bikes[i] = Some((key, paint, bike));
        } else if let Some((_, _, e)) = current
            && let Ok(mut t) = transforms.get_mut(*e)
        {
            t.translation = pose.0;
            t.rotation = pose.1;
        }
    }
}

fn place_camera(
    view: Res<WorkshopView>,
    scene: Res<Scene>,
    mut cams: Query<(&mut Transform, &mut Projection)>,
) {
    let Some(cam) = scene.camera else { return };
    if let Ok((mut t, mut p)) = cams.get_mut(cam) {
        *t = Transform::from_translation(view.eye).looking_at(view.target, Vec3::Y);
        if let Projection::Perspective(pp) = &mut *p
            && pp.fov != view.fov_y
        {
            pp.fov = view.fov_y;
        }
    }
}

/// The ride window at [`RIDE_WINDOW_FPS`]: the world camera renders only
/// some frames.
fn ride_window(
    time: Res<Time>,
    view: Res<WorkshopView>,
    mut scene: ResMut<Scene>,
    mut cams: Query<&mut Camera, With<FollowCamera>>,
) {
    if !scene.open {
        return;
    }
    scene.since_frame += time.delta_secs();
    let due = scene.since_frame >= 1.0 / RIDE_WINDOW_FPS;
    if due {
        scene.since_frame = 0.0;
    }
    for mut c in &mut cams {
        let want = view.ride_window && due;
        if c.is_active != want {
            c.is_active = want;
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a Bevy system
fn draw_ui(
    mut commands: Commands,
    view: Res<WorkshopView>,
    scene: Res<Scene>,
    cams: Query<(&Camera, &GlobalTransform)>,
    mut texts: ParamSet<(
        Query<&mut Text, With<StatusText>>,
        Query<&mut Text, With<HeartsText>>,
        Query<&mut Text, With<HintText>>,
        Query<&mut Text, With<CaptionSpan>>,
    )>,
    mut nodes: ParamSet<(
        Query<(Entity, &mut Node), With<Panel>>,
        Query<(Entity, &mut Node), With<DialogBox>>,
        Query<&mut Node, With<RideWindow>>,
        Query<&mut Node, With<BackButton>>,
        Query<&mut Node, With<CaptionText>>,
    )>,
    cards: Query<Entity, With<Cards>>,
    markers: Query<Entity, With<Markers>>,
    mut last: Local<Option<WorkshopView>>,
) {
    if !view.open || scene.ui.is_none() {
        *last = None;
        return;
    }
    let font = |vh: f32| TextFont {
        font_size: FontSize::Vh(vh),
        ..default()
    };
    if let Ok(mut t) = texts.p0().single_mut() {
        t.0.clone_from(&view.status);
    }
    if let Ok(mut t) = texts.p1().single_mut() {
        t.0.clone_from(&view.hearts);
    }
    if let Ok(mut t) = texts.p2().single_mut() {
        t.0.clone_from(&view.hint);
    }
    if let Ok(mut n) = nodes.p4().single_mut() {
        n.display = if view.caption.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut t) = texts.p3().single_mut() {
        t.0 = view.caption.clone().unwrap_or_default();
    }
    if let Ok(mut n) = nodes.p2().single_mut() {
        n.display = if view.ride_window && view.picker.is_none() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut n) = nodes.p3().single_mut() {
        n.display = if view.touch {
            Display::Flex
        } else {
            Display::None
        };
    }
    // Cards and markers follow the camera: rebuilt every frame (a few nodes).
    let cam = scene.camera.and_then(|c| cams.get(c).ok());
    let project = |p: Vec3| -> Option<Vec2> {
        let (c, t) = cam?;
        c.world_to_viewport(t, p).ok()
    };
    let layout = ocw_assets::workshop::layout(view.bikes.len());
    let width = cam
        .and_then(|(c, _)| c.logical_viewport_size())
        .map_or(1280.0, |s| s.x);
    // Name cards only in the room views (not at the stand or the picker).
    let rooms = view.picker.is_none() && view.hotspots.is_empty();
    if let Ok(cards) = cards.single() {
        commands.entity(cards).despawn_children();
        for (i, b) in view.bikes.iter().enumerate() {
            if !rooms || (i == view.stand && view.swap.is_none()) {
                continue;
            }
            let Some(h) = layout.hooks.get(i) else {
                continue;
            };
            let Some(at) = project(h.card) else { continue };
            let focused = view.focus == Some(i);
            let lines = if focused {
                b.card.clone()
            } else {
                b.card.iter().take(1).cloned().collect()
            };
            commands.entity(cards).with_children(|c| {
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(at.x - 115.0),
                        top: px(at.y - 14.0 - 26.0 * (lines.len() as f32 - 1.0)),
                        width: px(230.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        padding: UiRect::all(px(4.0)),
                        border: UiRect::all(px(if focused { 3.0 } else { 1.0 })),
                        border_radius: BorderRadius::all(px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(
                        0.95,
                        0.93,
                        0.88,
                        if focused { 0.95 } else { 0.8 },
                    )),
                    BorderColor::all(if focused {
                        Color::srgb(1.0, 0.78, 0.24)
                    } else {
                        Color::srgba(0.0, 0.0, 0.0, 0.3)
                    }),
                ))
                .with_children(|card| {
                    for (k, l) in lines.iter().enumerate() {
                        card.spawn((
                            Text::new(l.clone()),
                            font(if k == 0 { 2.0 } else { 1.7 }),
                            TextColor(Color::srgb(0.1, 0.1, 0.12)),
                        ));
                    }
                });
            });
        }
        // The "+" hook.
        if let Some(h) = layout.hooks.last()
            && rooms
            && let Some(at) = project(h.card)
        {
            commands.entity(cards).with_children(|c| {
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(at.x - 40.0),
                        top: px(at.y - 14.0),
                        width: px(80.0),
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(px(if view.plus_focused { 3.0 } else { 1.0 })),
                        border_radius: BorderRadius::all(px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.95, 0.93, 0.88, 0.7)),
                    BorderColor::all(if view.plus_focused {
                        Color::srgb(1.0, 0.78, 0.24)
                    } else {
                        Color::srgba(0.0, 0.0, 0.0, 0.3)
                    }),
                    children![(
                        Text::new("+"),
                        font(2.6),
                        TextColor(Color::srgb(0.1, 0.1, 0.12))
                    )],
                ));
            });
        }
    }
    if let Ok(markers) = markers.single() {
        commands.entity(markers).despawn_children();
        for h in &view.hotspots {
            let Some(at) = project(h.pos) else { continue };
            let r = if h.focused { 13.0 } else { 9.0 };
            commands.entity(markers).with_children(|m| {
                m.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(at.x - r),
                        top: px(at.y - r),
                        width: px(2.0 * r),
                        height: px(2.0 * r),
                        border: UiRect::all(px(2.0)),
                        border_radius: BorderRadius::all(px(r)),
                        ..default()
                    },
                    BackgroundColor(if h.focused {
                        Color::srgba(1.0, 0.78, 0.24, 0.95)
                    } else {
                        Color::srgba(0.35, 0.85, 1.0, 0.75)
                    }),
                    BorderColor::all(Color::WHITE),
                ));
                if h.focused {
                    m.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            // Left of the marker on the right of the window.
                            left: if at.x > width * 0.6 {
                                Val::Auto
                            } else {
                                px(at.x + 20.0)
                            },
                            right: if at.x > width * 0.6 {
                                px(width - at.x + 20.0)
                            } else {
                                Val::Auto
                            },
                            max_width: px(0.45 * width),
                            top: px(at.y - 16.0),
                            padding: UiRect::axes(px(10.0), px(5.0)),
                            border_radius: BorderRadius::all(px(6.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.85)),
                        children![(
                            Text::new(h.label.clone()),
                            font(2.6),
                            TextColor(Color::WHITE)
                        )],
                    ));
                }
            });
        }
    }
    // The picker and the dialog: rebuilt when they change.
    let changed = last.as_ref().is_none_or(|l| l.picker != view.picker);
    if changed {
        if let Ok((panel, mut n)) = nodes.p0().single_mut() {
            n.display = if view.picker.is_some() {
                Display::Flex
            } else {
                Display::None
            };
            commands.entity(panel).despawn_children();
            if let Some(p) = &view.picker {
                spawn_picker(&mut commands, panel, p);
            }
        }
        if let Ok((dialog, mut n)) = nodes.p1().single_mut() {
            let d = view.picker.as_ref().and_then(|p| p.dialog.as_ref());
            n.display = if d.is_some() {
                Display::Flex
            } else {
                Display::None
            };
            commands.entity(dialog).despawn_children();
            if let Some(d) = d {
                spawn_dialog(&mut commands, dialog, d);
            }
        }
    }
    *last = Some(view.clone());
}

const FOCUS: Color = Color::srgb(1.0, 0.78, 0.24);

fn spawn_picker(commands: &mut Commands, panel: Entity, p: &PickerView) {
    let font = |vh: f32| TextFont {
        font_size: FontSize::Vh(vh),
        ..default()
    };
    // At most this many rows around the focus (ten-foot sizes).
    const SHOWN: usize = 8;
    let start = p
        .focus
        .saturating_sub(SHOWN / 2)
        .min(p.rows.len().saturating_sub(SHOWN));
    commands.entity(panel).with_children(|c| {
        c.spawn((
            Text::new(p.title.clone()),
            font(4.2),
            TextColor(Color::WHITE),
        ));
        for (i, r) in p.rows.iter().enumerate().skip(start).take(SHOWN) {
            if let Some(g) = &r.group {
                c.spawn((
                    Text::new(g.clone()),
                    font(2.4),
                    TextColor(Color::srgb(0.7, 0.72, 0.76)),
                ));
            }
            let focused = i == p.focus;
            c.spawn((
                Node {
                    padding: UiRect::axes(vh(1.2), vh(0.5)),
                    border: UiRect::all(vh(0.35)),
                    border_radius: BorderRadius::all(vh(0.8)),
                    justify_content: JustifyContent::SpaceBetween,
                    column_gap: vh(1.0),
                    min_height: px(48),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(if focused {
                    Color::srgba(1.0, 0.78, 0.24, 0.22)
                } else {
                    Color::srgba(1.0, 1.0, 1.0, 0.06)
                }),
                BorderColor::all(if focused { FOCUS } else { Color::NONE }),
                children![
                    (
                        Text::new(r.left.clone()),
                        font(2.0),
                        TextColor(Color::WHITE)
                    ),
                    (
                        Text::new(r.right.clone()),
                        font(2.0),
                        TextColor(if r.locked {
                            Color::srgb(0.75, 0.55, 0.55)
                        } else {
                            Color::srgb(1.0, 0.6, 0.7)
                        })
                    )
                ],
            ))
            .observe(
                move |_: On<PointerOver>, mut ptr: ResMut<WorkshopPointer>| {
                    ptr.0.push(WorkshopPointerEvent::Row(i));
                },
            )
            .observe(
                move |click: On<PointerClick>, mut ptr: ResMut<WorkshopPointer>| {
                    if click.button == PointerButton::Primary {
                        ptr.0.push(WorkshopPointerEvent::RowClick(i));
                    }
                },
            );
        }
        let cards: Vec<&Vec<String>> = match &p.compare {
            Some(c) => vec![c],
            None => vec![&p.detail],
        };
        for card in cards {
            c.spawn((
                Node {
                    margin: UiRect::top(vh(1.0)),
                    padding: UiRect::all(vh(1.2)),
                    flex_direction: FlexDirection::Column,
                    row_gap: vh(0.4),
                    border_radius: BorderRadius::all(vh(0.8)),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
            ))
            .with_children(|d| {
                for (k, l) in card.iter().enumerate() {
                    d.spawn((
                        Text::new(l.clone()),
                        font(if k == 0 { 2.6 } else { 2.2 }),
                        TextColor(if l.ends_with("better") {
                            Color::srgb(0.55, 0.9, 0.6)
                        } else if l.ends_with("worse") {
                            Color::srgb(0.95, 0.6, 0.55)
                        } else {
                            Color::srgb(0.88, 0.9, 0.93)
                        }),
                    ));
                }
            });
        }
    });
}

fn spawn_dialog(commands: &mut Commands, root: Entity, d: &DialogView) {
    let font = |vh: f32| TextFont {
        font_size: FontSize::Vh(vh),
        ..default()
    };
    commands.entity(root).with_children(|c| {
        c.spawn((
            Node {
                width: vh(90.0),
                max_width: vw(90.0),
                padding: UiRect::all(vh(3.0)),
                flex_direction: FlexDirection::Column,
                row_gap: vh(1.4),
                border: UiRect::all(vh(0.3)),
                border_radius: BorderRadius::all(vh(1.5)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.95)),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.3)),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(d.title.clone()),
                font(4.4),
                TextColor(Color::WHITE),
            ));
            b.spawn((
                Text::new(d.text.clone()),
                font(3.0),
                TextColor(Color::srgb(0.8, 0.82, 0.86)),
            ));
            for (i, label) in d.buttons.iter().enumerate() {
                let focused = i == d.focus;
                b.spawn((
                    Node {
                        padding: UiRect::axes(vh(2.0), vh(0.8)),
                        border: UiRect::all(vh(0.45)),
                        border_radius: BorderRadius::all(vh(1.0)),
                        min_height: px(48),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(if focused {
                        Color::srgba(1.0, 0.78, 0.24, 0.22)
                    } else {
                        Color::srgba(1.0, 1.0, 1.0, 0.07)
                    }),
                    BorderColor::all(if focused { FOCUS } else { Color::NONE }),
                    children![(Text::new(label.clone()), font(3.6), TextColor(Color::WHITE))],
                ))
                .observe(
                    move |click: On<PointerClick>, mut ptr: ResMut<WorkshopPointer>| {
                        if click.button == PointerButton::Primary {
                            ptr.0.push(WorkshopPointerEvent::Button(i));
                        }
                    },
                );
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bikes_move_between_hook_and_stand_in_an_arc() {
        let layout = ocw_assets::workshop::layout(6);
        let models = BikeModels::default();
        let bike = models.get(ProfileId::Road, &[]);
        let stand = layout.stand_bike;
        assert_eq!(bike_pose(&layout, 0, 0, None, &bike), stand);
        let hook = layout.hooks[2].bike_pose(bike.wheel_radius, bike.wheelbase);
        assert_eq!(bike_pose(&layout, 2, 0, None, &bike), hook);
        // Mid-swap both are lifted above the straight line.
        let mid = bike_pose(&layout, 2, 0, Some((0, 2, 0.5)), &bike);
        assert!(mid.0.y > (hook.0.y + stand.0.y) / 2.0 + 0.2);
        let end = bike_pose(&layout, 2, 0, Some((0, 2, 1.0)), &bike);
        assert!(end.0.distance(stand.0) < 1e-4);
        let back = bike_pose(&layout, 0, 0, Some((0, 2, 1.0)), &bike);
        let own_hook = layout.hooks[0].bike_pose(bike.wheel_radius, bike.wheelbase);
        assert!(back.0.distance(own_hook.0) < 1e-4);
    }

    #[test]
    fn built_bikes_have_their_hotspots_and_are_cached() {
        let models = BikeModels::default();
        let a = models.get(ProfileId::Gravel, &["tyre-gravel-40".into()]);
        let b = models.get(
            ProfileId::Gravel,
            &["tyre-gravel-40".into(), "tyre-gravel-40".into()],
        );
        assert!(Arc::ptr_eq(&a, &b), "one build per key");
        assert_eq!(a.anchors.len(), ocw_assets::bikes::HOTSPOT_SLOTS.len());
        let (lo, hi) = a.bounds;
        for (_, p) in &a.anchors {
            assert!(
                p.cmpge(lo - Vec3::splat(0.2)).all() && p.cmple(hi + Vec3::splat(0.2)).all(),
                "{p}"
            );
        }
        let c = bike_corners((Vec3::X, Quat::IDENTITY), &a);
        assert!((c[0] - (lo + Vec3::X)).length() < 1e-5);
    }
}
