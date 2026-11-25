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
//!
//! The changing corner (W4, spec 4C): the rider stands in front of the
//! mirror ([`RiderShow`]: the look with the character and the worn kit,
//! the picker's preview applied), seen over the shoulder and in the
//! mirror – a planar reflection: a second camera at the reflection of the
//! workshop camera renders the room at half resolution into an image
//! shown on the glass ([`mirror_cam`], [`mirror_uv`]; off with
//! `workshop.mirror off`: a direct front view instead). *Try on the bike*
//! puts the rider on the stand bike, pedalling slowly.

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
use ocw_assets::workshop::Layout;
use ocw_core::{ProfileId, Rgb};

use crate::camera::FollowCamera;
use crate::gear::{self, GearMaterial};

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
    /// The part's damage (spec §14 Phase 4 W6, Phase 12 Q6): the marker
    /// takes the status colour (the label says the condition in words);
    /// `None`: as new.
    pub damage: Option<ocw_sim::damage::Status>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PickerRowView {
    /// A group header shown above this row ("Equipped", "Owned", "Shop").
    pub group: Option<String>,
    pub left: String,
    pub right: String,
    pub locked: bool,
    /// The tile (spec 4B *Tiles*, when the picker shows tiles): the
    /// part's thumbnail …
    pub thumb: Option<crate::thumbs::ThumbKey>,
    /// … or a colour swatch (the mirror's skin and hair colours, 4N).
    pub swatch: Option<Rgb>,
    /// … its gain ("+0.8 % flat · −0.2 % climb") and weight ("−240 g")
    /// against the slot's base part, and whether the caps cut it.
    pub gain: String,
    pub weight: String,
    pub capped: bool,
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
    /// The rows as a grid of tiles with thumbnails and gains (spec 4B
    /// *Tiles*; the grid's model is `ocw_client::part_tiles`); `None`: a
    /// list.
    pub tiles: Option<TilesView>,
    /// A line under the title (the whole bike against the stock bike).
    pub subtitle: Option<String>,
    /// A row of colour swatches above the tiles (spec §14 Phase 4, 4N
    /// Q30 (a): the kit's own colours of shorts, socks, shoes, gloves).
    pub swatches: Option<SwatchRowView>,
}

/// A kit picker's swatch row as shown (4N Q30 (a)): each swatch the
/// colour itself, the selected one outlined.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SwatchRowView {
    /// "Colour of the gloves: standard (black)".
    pub label: String,
    pub colours: Vec<Rgb>,
    pub selected: usize,
    /// The focus is on the row (not on a tile).
    pub focused: bool,
    /// The swatch of the standard colour (the model's own), marked "std"
    /// – it may look like a palette colour.
    pub standard: Option<usize>,
}

/// The tile grid as shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TilesView {
    /// Tiles per row.
    pub cols: usize,
    /// The first tile shown and how many (whole rows, scrolled to the
    /// focus).
    pub first: usize,
    pub shown: usize,
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
    /// The rider in the changing corner (W4).
    pub rider: Option<RiderShow>,
    /// The mirror's reflection (off: low graphics quality).
    pub mirror: bool,
    /// The condition card (spec §14 Phase 4 W6): the stand bike's damaged
    /// parts with their repair prices and *Repair all*, or at the mirror
    /// the rider's injury and the time to heal; the first line is the
    /// title. Empty: no card.
    pub condition: Vec<String>,
    /// The helmet off (spec §14 Phase 4, 4N Q28): the worn kit's variants
    /// while it hangs on the hook beside the mirror; `None`: worn.
    pub helmet_on_hook: Option<Vec<String>>,
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
            rider: None,
            mirror: true,
            condition: Vec::new(),
            helmet_on_hook: None,
        }
    }
}

/// Pointer events on the workshop's panels since the app last read them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkshopPointerEvent {
    Row(usize),
    RowClick(usize),
    /// Over / clicked swatch `i` of the swatch row (4N Q30 (a)).
    Swatch(usize),
    SwatchClick(usize),
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
    let low = ocw_assets::shipped::rider_low_model();
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
        if !app.is_plugin_added::<crate::thumbs::ThumbnailPlugin>() {
            app.add_plugins(crate::thumbs::ThumbnailPlugin);
        }
        app.init_resource::<Scene>()
            .add_systems(
                Update,
                (
                    open_close,
                    sync_bikes,
                    sync_rider,
                    sync_helmet_hook,
                    draw_ui,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, (place_camera, place_mirror, ride_window));
    }
}

/// The open scene's entities.
#[derive(Resource, Default)]
struct Scene {
    open: bool,
    /// The number of bikes the room was built for (its hooks; a bike
    /// bought or sold at the bench rebuilds it, spec 4D).
    built_for: usize,
    camera: Option<Entity>,
    root: Option<Entity>,
    ui: Option<Entity>,
    image: Option<Handle<Image>>,
    /// UI roots of the ride hidden while open, with their visibility,
    /// and the HUD switch (the HUD sets its panels' visibility itself).
    hidden: Vec<(Entity, Visibility)>,
    hud_was: Option<bool>,
    /// Spawned bikes: (key, paint, entity, the model's node entities).
    bikes: Vec<Option<SpawnedBike>>,
    /// The rider of the changing corner.
    rider: Option<ShownRider>,
    /// The mirror's camera, and its picture on the glass (mesh, material).
    mirror_cam: Option<Entity>,
    mirror_quad: Option<(Handle<Mesh>, Handle<StandardMaterial>)>,
    /// Gear materials by surface (spec §14 Phase 8 T6).
    materials: HashMap<Vec<u32>, Handle<GearMaterial>>,
    meshes: HashMap<usize, NodeMeshes>,
    since_frame: f32,
}

/// A spawned bike: (key, paint, entity, the model's node entities).
type SpawnedBike = (BikeKey, Rgb, Entity, Vec<Entity>);
/// A model's meshes on the GPU: `(mesh, material index)` per node.
pub(crate) type NodeMeshes = Vec<Vec<(Handle<Mesh>, usize)>>;
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

/// Despawns the open scene and gives the ride back its window and UI.
fn close_scene(
    commands: &mut Commands,
    scene: &mut Scene,
    world_cams: &mut Query<(Entity, &mut Camera, &mut RenderTarget), With<FollowCamera>>,
) {
    despawn_scene(commands, scene);
    for (e, v) in scene.hidden.drain(..) {
        if let Ok(mut c) = commands.get_entity(e) {
            c.insert(v);
        }
    }
    for (_, mut cam, mut target) in world_cams.iter_mut() {
        *target = RenderTarget::Window(WindowRef::Primary);
        cam.is_active = true;
    }
}

/// Despawns the scene's cameras, room, bikes, rider and UI.
fn despawn_scene(commands: &mut Commands, scene: &mut Scene) {
    for e in [
        scene.camera.take(),
        scene.root.take(),
        scene.ui.take(),
        scene.mirror_cam.take(),
    ]
    .into_iter()
    .flatten()
    {
        commands.entity(e).despawn();
    }
    scene.rider = None;
    scene.bikes.clear();
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
    // A bike bought or sold while open: the room again with its hooks (as
    // if closed and opened at once; spec 4D).
    let rebuild = view.open && scene.open && scene.built_for != view.bikes.len();
    if view.open == scene.open && !rebuild {
        return;
    }
    if rebuild {
        // The ride's UI stays hidden and the HUD off meanwhile.
        despawn_scene(&mut commands, &mut scene);
    }
    scene.open = view.open;
    if let Some(h) = hud.as_mut().filter(|_| !rebuild) {
        if view.open {
            scene.hud_was = Some(h.0);
            h.0 = false;
        } else if let Some(was) = scene.hud_was.take() {
            h.0 = was;
        }
    }
    if !view.open {
        close_scene(&mut commands, &mut scene, &mut world_cams);
        return;
    }
    scene.built_for = view.bikes.len();
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
    // The ride's overlays hide (on a rebuild they are hidden already).
    if !rebuild {
        scene.hidden = roots.iter().map(|(e, v)| (e, *v)).collect();
        for (e, _) in &scene.hidden {
            commands.entity(*e).insert(Visibility::Hidden);
        }
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
            RenderLayers::from_layers(&[LAYER, MIRROR_LAYER]),
        ))
        .id();
    scene.camera = Some(camera);
    let layout_m = ocw_assets::workshop::layout(view.bikes.len());
    if view.mirror {
        // The mirror (spec 4C): a second camera at half resolution.
        let (glass, half) = layout_m.mirror_glass();
        let mh = (h / 2).max(64);
        let mw = ((mh as f32) * half.x / half.y).round().max(32.0) as u32;
        let mimage = images.add(Image::new_target_texture(
            mw,
            mh,
            TextureFormat::Bgra8UnormSrgb,
            None,
        ));
        let mcam = commands
            .spawn((
                Camera3d::default(),
                Camera {
                    order: 0,
                    clear_color: ClearColorConfig::Custom(Color::srgb(0.12, 0.11, 0.10)),
                    ..default()
                },
                RenderTarget::Image(mimage.clone().into()),
                Projection::Perspective(PerspectiveProjection::default()),
                Transform::default(),
                AmbientLight {
                    color: Color::srgb(1.0, 0.93, 0.85),
                    brightness: 260.0,
                    ..default()
                },
                bevy::camera::Exposure { ev100: 7.5 },
                layer(),
            ))
            .id();
        scene.mirror_cam = Some(mcam);
        let quad = materials.add(StandardMaterial {
            base_color_texture: Some(mimage),
            unlit: true,
            cull_mode: None,
            ..default()
        });
        scene.mirror_quad = Some((meshes.add(mirror_mesh(glass, half)), quad));
    }
    // The room.
    // The room from its piece files (spec 15M), merged per group and
    // material: drawn as one mesh per material and group, as before the
    // pieces were files (as separate pieces the mirror camera's
    // reflection lost the far wall).
    let room = ocw_assets::workshop::room_merged(&layout);
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
    if let Some((mesh, mat)) = scene.mirror_quad.take() {
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            ChildOf(root),
            RenderLayers::layer(MIRROR_LAYER),
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

/// A built bike's (or the rider's) meshes on the GPU, per node, with the
/// gear shader's rest coordinates (spec §14 Phase 8 T6).
pub(crate) fn upload(model: &Model, meshes: &mut Assets<Mesh>, rider: bool) -> NodeMeshes {
    model
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (rest, region) = gear::rest_frame(model, i, rider);
            n.primitives
                .iter()
                .map(|p| (meshes.add(gear::gear_mesh(p, rest, region)), p.material))
                .collect()
        })
        .collect()
}

/// The gear material of a part (spec §14 Phase 8 T6): its kind from slot
/// and node, its colours from `colours`; the light set's lenses glow.
pub(crate) fn part_material(
    cache: &mut HashMap<Vec<u32>, Handle<GearMaterial>>,
    materials: &mut Assets<GearMaterial>,
    slot: &str,
    node: &str,
    model_color: [f32; 4],
    colours: &crate::riders::AvatarColours,
) -> Handle<GearMaterial> {
    let glow = slot.starts_with("light_");
    let s = gear::surface(slot, node, model_color, colours);
    cache
        .entry(s.key(glow))
        .or_insert_with(|| materials.add(s.material(glow)))
        .clone()
}

/// The colours of a bike in `paint` (the parts keep the model's).
pub(crate) fn bike_colours(paint: Rgb) -> crate::riders::AvatarColours {
    let look = ocw_core::RiderLook {
        frame: paint,
        ..Default::default()
    };
    crate::riders::avatar_colours(ocw_core::RiderId(0), true, Some(&look))
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn sync_bikes(
    mut commands: Commands,
    view: Res<WorkshopView>,
    models: Res<BikeModels>,
    mut scene: ResMut<Scene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GearMaterial>>,
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
        let fresh = current.is_none_or(|(k, p, _, _)| *k != key || *p != paint);
        if fresh {
            if let Some((_, _, e, _)) = scene.bikes[i].take() {
                commands.entity(e).despawn();
            }
            let model = &built.model;
            let ptr = Arc::as_ptr(&built) as usize;
            let handles = scene
                .meshes
                .entry(ptr)
                .or_insert_with(|| upload(model, &mut meshes, false))
                .clone();
            let colours = bike_colours(paint);
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
                    let material = part_material(
                        &mut scene.materials,
                        &mut materials,
                        &m.name,
                        &node.name,
                        m.color,
                        &colours,
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
            scene.bikes[i] = Some((key, paint, bike, ents));
        } else if let Some((_, _, e, _)) = current
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
    mut last_ui: Local<Option<Entity>>,
    mut last_aspect: Local<f32>,
    mut thumbs: ResMut<crate::thumbs::Thumbnails>,
    mut images: ResMut<Assets<Image>>,
) {
    if !view.open || scene.ui.is_none() {
        *last = None;
        return;
    }
    // A rebuilt scene (a bike bought, spec 4D): its new UI drawn whole.
    if *last_ui != scene.ui {
        *last_ui = scene.ui;
        *last = None;
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
    let rooms = view.picker.is_none() && view.hotspots.is_empty() && view.rider.is_none();
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
        // The condition card (W6): bottom left, while no picker is open.
        if !view.condition.is_empty() && view.picker.is_none() {
            commands.entity(cards).with_children(|c| {
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: vw(2.5),
                        bottom: vh(16.0),
                        max_width: vw(42.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: vh(0.4),
                        padding: UiRect::axes(vh(1.6), vh(1.1)),
                        border: UiRect::all(px(2.0)),
                        border_radius: BorderRadius::all(vh(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.82)),
                    BorderColor::all(Color::srgba(0.98, 0.52, 0.12, 0.9)),
                ))
                .with_children(|card| {
                    for (k, l) in view.condition.iter().enumerate() {
                        card.spawn((
                            Text::new(l.clone()),
                            font(if k == 0 { 2.6 } else { 2.1 }),
                            TextColor(if k == 0 {
                                Color::srgb(1.0, 0.78, 0.24)
                            } else {
                                Color::WHITE
                            }),
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
                    BackgroundColor(match (h.focused, h.damage) {
                        (true, _) => Color::srgba(1.0, 0.78, 0.24, 0.95),
                        // A damaged part: its status colour (W6).
                        (false, Some(s)) => crate::damage_hud::status_color(s).with_alpha(0.9),
                        (false, None) => Color::srgba(0.35, 0.85, 1.0, 0.75),
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
    // The picker and the dialog: rebuilt when they change (or the window's
    // shape: the card is fitted to the panel).
    let aspect = cam
        .and_then(|(c, _)| c.logical_viewport_size())
        .map_or(16.0 / 9.0, |s| s.x / s.y.max(1.0));
    let resized = (aspect - *last_aspect).abs() > 0.01;
    *last_aspect = aspect;
    let changed = resized || last.as_ref().is_none_or(|l| l.picker != view.picker);
    if changed {
        if let Ok((panel, mut n)) = nodes.p0().single_mut() {
            n.display = if view.picker.is_some() {
                Display::Flex
            } else {
                Display::None
            };
            commands.entity(panel).despawn_children();
            if let Some(p) = &view.picker {
                spawn_picker(&mut commands, panel, p, aspect, &mut thumbs, &mut images);
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

/// The mirror's render layer: the mirror's picture is seen by the
/// workshop camera only, never by the mirror camera itself.
pub const MIRROR_LAYER: usize = 8;

/// The mirror camera of a workshop camera at `eye` (spec 4C: a planar
/// reflection): the eye mirrored at the glass plane (x = glass.x, facing
/// −X), looking straight at the glass (so its picture plane is the
/// glass's), its near plane on the glass (nothing behind the mirror), and
/// a sub view of a frustum centred on the eye's foot point that is
/// exactly the glass rectangle. The picture maps onto the glass with
/// [`mirror_uv`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MirrorCam {
    pub eye: Vec3,
    /// Vertical field of view of the full frustum (rad).
    pub fov: f32,
    pub near: f32,
    pub sub: bevy::camera::SubCameraView,
}

/// Millimetres per unit of the sub view (its sizes are integers).
const MIRROR_GRID: f32 = 1000.0;

pub fn mirror_cam(eye: Vec3, glass: Vec3, half: Vec2) -> Option<MirrorCam> {
    let d = glass.x - eye.x;
    if d < 0.05 {
        // At or behind the glass: nothing to see.
        return None;
    }
    let mirrored = Vec3::new(glass.x + d, eye.y, eye.z);
    let (zmin, zmax) = (glass.z - half.x, glass.z + half.x);
    let (ymin, ymax) = (glass.y - half.y, glass.y + half.y);
    let hw = (eye.z - zmin).abs().max((zmax - eye.z).abs()) + 0.01;
    let hh = (eye.y - ymin).abs().max((ymax - eye.y).abs()) + 0.01;
    // The camera's right is −Z (it looks along −X), screen y down.
    let full = UVec2::new(
        (2.0 * hw * MIRROR_GRID).ceil() as u32,
        (2.0 * hh * MIRROR_GRID).ceil() as u32,
    );
    let hw = full.x as f32 / (2.0 * MIRROR_GRID);
    let hh = full.y as f32 / (2.0 * MIRROR_GRID);
    Some(MirrorCam {
        eye: mirrored,
        fov: 2.0 * (hh / d).atan(),
        near: d + 0.005,
        sub: bevy::camera::SubCameraView {
            full_size: full,
            offset: Vec2::new(
                (eye.z - zmax + hw) * MIRROR_GRID,
                (eye.y + hh - ymax) * MIRROR_GRID,
            ),
            size: UVec2::new(
                (2.0 * half.x * MIRROR_GRID).round() as u32,
                (2.0 * half.y * MIRROR_GRID).round() as u32,
            ),
        },
    })
}

/// Texture coordinates of the glass point `p` (x ignored) in the mirror
/// camera's picture: left of the picture = the glass's +Z edge (the
/// camera looks back into the room), top = the top edge.
pub fn mirror_uv(glass: Vec3, half: Vec2, p: Vec3) -> Vec2 {
    Vec2::new(
        (glass.z + half.x - p.z) / (2.0 * half.x),
        (glass.y + half.y - p.y) / (2.0 * half.y),
    )
}

/// The glass's picture: a quad just in front of the glass, facing −X,
/// with the [`mirror_uv`] mapping.
fn mirror_mesh(glass: Vec3, half: Vec2) -> Mesh {
    let x = glass.x - 0.002;
    let corners = [
        Vec3::new(x, glass.y - half.y, glass.z - half.x),
        Vec3::new(x, glass.y - half.y, glass.z + half.x),
        Vec3::new(x, glass.y + half.y, glass.z + half.x),
        Vec3::new(x, glass.y + half.y, glass.z - half.x),
    ];
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        corners.iter().map(|c| c.to_array()).collect::<Vec<_>>(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[-1.0f32, 0.0, 0.0]; 4]);
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        corners
            .iter()
            .map(|c| mirror_uv(glass, half, *c).to_array())
            .collect::<Vec<_>>(),
    );
    m.insert_indices(Indices::U32(vec![0, 2, 1, 0, 3, 2]));
    m
}

/// The rider in the changing corner (spec 4C, W4).
#[derive(Debug, Clone, PartialEq)]
pub struct RiderShow {
    /// The look with the character (the picker's preview applied).
    pub look: ocw_core::RiderLook,
    /// The rider variants of the worn kit (`ocw_assets::rider::VARIANTS`).
    pub kit: Vec<String>,
    /// *Try on the bike*: on the stand bike, pedalling slowly.
    pub on_stand: bool,
    /// Turned about the vertical (rad).
    pub yaw: f32,
}

/// Cadence of *Try on the bike* (rpm): slow pedalling.
pub const TRY_ON_RPM: f64 = 45.0;

/// The spawned rider of the changing corner.
struct ShownRider {
    key: RiderShow,
    root: Entity,
    /// The rider model's node entities (PLACEHOLDER for hidden ones).
    nodes: Vec<Entity>,
    body: ocw_assets::pose::BodyFit,
    crank: f32,
    /// The entity it hangs under (the room, or the stand bike).
    parent: Option<Entity>,
}

/// Spawns the rider model with `look` and the worn `kit` on `layer`: its
/// root (scaled to the body height, `scale`) and the node entities
/// (`Entity::PLACEHOLDER` for the hidden ones). With `pose`, the nodes
/// take that standing pose at once (the thumbnails, which are never
/// animated); else the caller poses them. Shared by the changing corner
/// and the character thumbnails (spec §14 Phase 4, 4N).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_rider_model(
    commands: &mut Commands,
    look: &ocw_core::RiderLook,
    kit: &[String],
    layer: RenderLayers,
    scale: f32,
    pose: Option<&ocw_assets::pose::Pose>,
    meshes: &mut Assets<Mesh>,
    mesh_cache: &mut HashMap<usize, NodeMeshes>,
    materials: &mut Assets<GearMaterial>,
    mat_cache: &mut HashMap<Vec<u32>, Handle<GearMaterial>>,
) -> (Entity, Vec<Entity>) {
    let rider_model = ocw_assets::shipped::rider_model();
    let c = look.character.validated();
    let mut colours = crate::riders::avatar_colours(ocw_core::RiderId(0), true, Some(look));
    colours.gloved_tips = ocw_assets::rider::hand::covers_fingertips(kit);
    let hands_shown = pose.map_or([ocw_assets::rider::hand::HandShape::Relaxed; 2], |p| {
        p.hand_shapes
    });
    let hidden: Vec<usize> = crate::riders::hidden_rider_nodes(Some(look), kit)
        .into_iter()
        .filter_map(|n| rider_model.node(n))
        .collect();
    let ptr = rider_model as *const Model as usize;
    let handles = mesh_cache
        .entry(ptr)
        .or_insert_with(|| upload(rider_model, meshes, true))
        .clone();
    let root = commands
        .spawn((Transform::default(), Visibility::default(), layer.clone()))
        .id();
    let posed: HashMap<&str, Quat> = pose
        .map(|p| p.rotations().into_iter().collect())
        .unwrap_or_default();
    let mut nodes: Vec<Entity> = vec![Entity::PLACEHOLDER; rider_model.nodes.len()];
    for (i, node) in rider_model.nodes.iter().enumerate() {
        if hidden.iter().any(|h| rider_model.is_below(i, *h)) {
            continue;
        }
        let parent = node.parent.map_or(root, |p| nodes[p]);
        let mut translation = ocw_assets::rider::node_translation(rider_model, i, &c);
        let mut rotation = node.rotation;
        if let Some(p) = pose {
            if node.name == ocw_assets::pose::nodes::PELVIS {
                (translation, rotation) = p.pelvis;
            } else if let Some(q) = posed.get(node.name.as_str()) {
                rotation = *q;
            }
        }
        let e = commands
            .spawn((
                Transform {
                    translation,
                    rotation,
                    scale: if node.parent.is_none() {
                        Vec3::splat(scale)
                    } else {
                        node.scale
                    },
                },
                match ocw_assets::rider::hand::of_node(&node.name) {
                    Some((shape, side)) if hands_shown[side] != shape => Visibility::Hidden,
                    _ => Visibility::Inherited,
                },
                ChildOf(parent),
                layer.clone(),
            ))
            .id();
        let (girth, shift) = ocw_assets::rider::girth_transform(rider_model, i, &c);
        for (mesh, mat) in &handles[i] {
            let m = &rider_model.materials[*mat];
            let material =
                part_material(mat_cache, materials, &m.name, &node.name, m.color, &colours);
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(shift).with_scale(girth),
                ChildOf(e),
                layer.clone(),
            ));
        }
        nodes[i] = e;
    }
    (root, nodes)
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn sync_rider(
    mut commands: Commands,
    time: Res<Time>,
    view: Res<WorkshopView>,
    models: Res<BikeModels>,
    mut scene: ResMut<Scene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GearMaterial>>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
) {
    if !view.open || scene.root.is_none() {
        return;
    }
    // A respawned stand bike took the rider on it with it.
    let stand_now = scene
        .bikes
        .get(view.stand)
        .and_then(|b| b.as_ref())
        .map(|b| b.2);
    if scene
        .rider
        .as_ref()
        .is_some_and(|r| r.key.on_stand && r.parent.is_some() && r.parent != stand_now)
    {
        scene.rider = None;
    }
    let Some(show) = view.rider.clone() else {
        if let Some(r) = scene.rider.take() {
            commands.entity(r.root).try_despawn();
            // The stand bike back without the rider's saddle and cranks.
            if r.key.on_stand
                && let Some(b) = scene.bikes.get_mut(view.stand)
                && let Some((_, _, e, _)) = b.take()
            {
                commands.entity(e).despawn();
            }
        }
        return;
    };
    let layout = ocw_assets::workshop::layout(view.bikes.len());
    let stand_bike = view
        .bikes
        .get(view.stand)
        .map(|b| models.get(b.profile, &b.variants));
    let fresh = scene.rider.as_ref().is_none_or(|r| {
        r.key.look != show.look || r.key.kit != show.kit || r.key.on_stand != show.on_stand
    });
    if fresh {
        let was_on_stand = scene.rider.as_ref().is_some_and(|r| r.key.on_stand);
        if let Some(r) = scene.rider.take() {
            commands.entity(r.root).try_despawn();
        }
        if was_on_stand != show.on_stand
            && let Some(b) = scene.bikes.get_mut(view.stand)
            && let Some((_, _, e, _)) = b.take()
        {
            // Respawned by `sync_bikes`: the saddle and cranks as built.
            commands.entity(e).despawn();
        }
        let rig = ocw_assets::shipped::rider_rig();
        let c = show.look.character.validated();
        let bike_rig = stand_bike
            .as_ref()
            .and_then(|b| ocw_assets::pose::BikeRig::from_model(&b.model).ok())
            .unwrap_or_else(|| ocw_assets::shipped::bike_rig(ProfileId::Road));
        let body = ocw_assets::pose::BodyFit::new(&rig.for_body(&c), &bike_rig, c.scale());
        let Scene {
            meshes: mesh_cache,
            materials: mat_cache,
            ..
        } = &mut *scene;
        let (root, nodes) = spawn_rider_model(
            &mut commands,
            &show.look,
            &show.kit,
            layer(),
            body.scale,
            None,
            &mut meshes,
            mesh_cache,
            &mut materials,
            mat_cache,
        );
        scene.rider = Some(ShownRider {
            key: show.clone(),
            root,
            nodes,
            body,
            crank: 0.0,
            parent: None,
        });
    }
    let root_entity = scene.root.expect("open");
    let stand = scene
        .bikes
        .get(view.stand)
        .and_then(|b| b.as_ref())
        .map(|(_, _, e, nodes)| (*e, nodes.clone()));
    let Some(r) = scene.rider.as_mut() else {
        return;
    };
    r.key.yaw = show.yaw;
    if !show.on_stand {
        // Standing in front of the mirror, the feet on the floor.
        let (feet, rot) = layout.mirror_rider();
        let rig = ocw_assets::shipped::rider_rig();
        let sole = rig.leg() - rig.cleat.y;
        if r.parent != Some(root_entity) {
            commands.entity(r.root).insert(ChildOf(root_entity));
            r.parent = Some(root_entity);
        }
        if let Ok(mut t) = transforms.get_mut(r.root) {
            *t = Transform::from_translation(feet + Vec3::Y * sole * r.body.scale)
                .with_rotation(rot * Quat::from_rotation_y(show.yaw));
        }
        // Standing at ease (spec 4C): the idle animation, not the rest
        // pose; in the rider's own (unscaled) frame.
        let p = ocw_assets::pose::idle_aged(
            &rig,
            time.elapsed_secs(),
            r.key.look.character.validated().age.idle_hunch(),
        );
        let rider_model = ocw_assets::shipped::rider_model();
        let node = |name: &str| {
            rider_model
                .node(name)
                .map(|i| r.nodes[i])
                .filter(|e| *e != Entity::PLACEHOLDER)
        };
        if let Some(e) = node(ocw_assets::pose::nodes::PELVIS)
            && let Ok(mut t) = transforms.get_mut(e)
        {
            t.translation = p.pelvis.0;
            t.rotation = p.pelvis.1;
        }
        for (name, q) in p.rotations() {
            if let Some(e) = node(name)
                && let Ok(mut t) = transforms.get_mut(e)
            {
                t.rotation = q;
            }
        }
        // The idle pose's relaxed hands (bug 2026-11-17: after *Try on the
        // bike* the grip shapes stayed on the standing rider).
        let hands: Vec<_> = ocw_assets::rider::hand::HandShape::ALL
            .into_iter()
            .flat_map(|s| [(s, 0), (s, 1)])
            .filter_map(|(s, side)| Some((s, side, node(ocw_assets::rider::hand::node(s, side))?)))
            .collect();
        crate::riders::show_hands(&hands, p.hand_shapes, &mut visibility);
        return;
    }
    // On the stand bike: in its frame, pedalling slowly (spec 4C).
    let Some((bike_e, bike_nodes)) = stand else {
        return;
    };
    let Some(built) = stand_bike else { return };
    if r.parent != Some(bike_e) {
        commands.entity(r.root).insert(ChildOf(bike_e));
        r.parent = Some(bike_e);
    }
    r.crank = ocw_assets::anim::advance_crank(r.crank, TRY_ON_RPM, time.delta_secs());
    let params = ocw_assets::pose::PoseParams {
        crank: r.crank,
        ..Default::default()
    };
    let p = ocw_assets::pose::pose(&r.body.rider, &r.body.bike, &r.body.fit, &params);
    let rider_model = ocw_assets::shipped::rider_model();
    let find = |name: &str| -> Option<Entity> {
        if let Some(i) = rider_model.node(name) {
            return (r.nodes[i] != Entity::PLACEHOLDER).then_some(r.nodes[i]);
        }
        built
            .model
            .node(name)
            .and_then(|i| bike_nodes.get(i).copied())
    };
    if let Ok(mut t) = transforms.get_mut(r.root) {
        *t = Transform::IDENTITY;
    }
    if let Some(e) = find(ocw_assets::pose::nodes::PELVIS)
        && let Ok(mut t) = transforms.get_mut(e)
    {
        t.translation = p.pelvis.0 / r.body.scale;
        t.rotation = p.pelvis.1;
    }
    for (name, q) in p.rotations() {
        if let Some(e) = find(name)
            && let Ok(mut t) = transforms.get_mut(e)
        {
            t.rotation = q;
        }
    }
    // The hands round the bar (spec §14 Phase 4, 4N, the hands).
    let hands: Vec<_> = ocw_assets::rider::hand::HandShape::ALL
        .into_iter()
        .flat_map(|s| [(s, 0), (s, 1)])
        .filter_map(|(s, side)| Some((s, side, find(ocw_assets::rider::hand::node(s, side))?)))
        .collect();
    crate::riders::show_hands(&hands, p.hand_shapes, &mut visibility);
    // The saddle at the rider's height; the rear wheel turning.
    if let Some(i) = built.model.node(bike_nodes::SADDLE)
        && let Some(e) = bike_nodes.get(i)
        && let Ok(mut t) = transforms.get_mut(*e)
    {
        t.translation =
            built.model.nodes[i].translation + r.body.bike.seat_dir() * r.body.saddle_offset;
    }
    if let Some(e) = find(bike_nodes::WHEEL_REAR)
        && let Ok(mut t) = transforms.get_mut(e)
    {
        t.rotation = Quat::from_rotation_x(-r.crank * 2.6);
    }
}

/// The helmet's hook beside the mirror and the helmet on it while the
/// editor shows the bare head (spec §14 Phase 4, 4N Q28).
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn sync_helmet_hook(
    mut commands: Commands,
    view: Res<WorkshopView>,
    mut scene: ResMut<Scene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GearMaterial>>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    mut state: Local<HookState>,
) {
    let Some(root) = scene.root.filter(|_| view.open) else {
        *state = (None, None);
        return;
    };
    let layout = ocw_assets::workshop::layout(view.bikes.len());
    let hook = layout.helmet_hook();
    // The peg (spawned with the room, despawned with it).
    if state.0.is_none_or(|e| commands.get_entity(e).is_err()) {
        let peg = commands
            .spawn((
                Mesh3d(meshes.add(Cylinder::new(0.012, 0.09))),
                MeshMaterial3d(std_materials.add(StandardMaterial {
                    base_color: Color::srgb(0.32, 0.3, 0.28),
                    metallic: 0.6,
                    perceptual_roughness: 0.4,
                    ..default()
                })),
                Transform::from_translation(hook + Vec3::X * 0.02)
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2 + 0.25)),
                ChildOf(root),
                layer(),
            ))
            .id();
        state.0 = Some(peg);
        state.1 = None;
    }
    let want = view.helmet_on_hook.clone();
    if state.1.as_ref().map(|(k, _)| k) == want.as_ref() {
        return;
    }
    if let Some((_, e)) = state.1.take() {
        commands.entity(e).try_despawn();
    }
    let Some(kit) = want else { return };
    let model = ocw_assets::shipped::rider_model();
    let ids: Vec<&str> = kit
        .iter()
        .map(String::as_str)
        .filter(|k| *k != ocw_assets::rider::HELMET_OFF)
        .collect();
    let shown = ocw_assets::rider::visible(&ocw_core::Character::default(), &ids);
    let Some((i, node)) = model.nodes.iter().enumerate().find(|(_, n)| {
        ocw_assets::helmet::Helmet::ALL
            .iter()
            .any(|h| h.node() == n.name && shown.contains(&h.node()))
    }) else {
        return;
    };
    let look = view.rider.as_ref().map(|r| r.look).unwrap_or_default();
    let colours = crate::riders::avatar_colours(ocw_core::RiderId(0), true, Some(&look));
    let ptr = model as *const Model as usize;
    let handles = scene
        .meshes
        .entry(ptr)
        .or_insert_with(|| upload(model, &mut meshes, true))
        .clone();
    let e = commands
        .spawn((
            hook_helmet_transform(model, i, hook, layout.bounds.1.x),
            Visibility::Inherited,
            ChildOf(root),
            layer(),
            HelmetOnHook,
        ))
        .id();
    // The shell's inside (bug 2026-11-17: hanging in front of the room the
    // helmet showed the wall through its face opening – the shell is drawn
    // from outside only, a head fills it on the rider): the dark liner.
    let liner = std_materials.add(StandardMaterial {
        base_color: Color::srgb(0.1, 0.1, 0.11),
        perceptual_roughness: 0.9,
        ..default()
    });
    for p in &node.primitives {
        if model.materials[p.material].name == "helmet" {
            commands.spawn((
                Mesh3d(meshes.add(plain_mesh(&inner_lining(p)))),
                MeshMaterial3d(liner.clone()),
                ChildOf(e),
                layer(),
            ));
        }
    }
    for (mesh, mat) in &handles[i] {
        let m = &model.materials[*mat];
        let material = part_material(
            &mut scene.materials,
            &mut materials,
            &m.name,
            &node.name,
            m.color,
            &colours,
        );
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            ChildOf(e),
            layer(),
        ));
    }
    state.1 = Some((kit, e));
}

/// How helmet node `helmet` of the rider model hangs on its hook at
/// `hook` on the wall at x = `wall` (spec §14 Phase 4, 4N Q28; bug
/// 2026-11-17: it hung half in the wall): upright – the crown up, tipped
/// back towards the wall a little –, its front towards the room (−X), the
/// skull's centre 7 cm under the peg (the peg hidden in the shell), the
/// back of the shell (a TT helmet's tail too) against the wall.
pub fn hook_helmet_transform(model: &Model, helmet: usize, hook: Vec3, wall: f32) -> Transform {
    let head = model.node(ocw_assets::pose::nodes::HEAD).expect("a head");
    let skull = ocw_assets::rider::skull_centre() - Vec3::from(model.world(head).translation);
    let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(0.35);
    // The shell's farthest point towards the wall, from the skull's centre.
    let back = model.nodes[helmet]
        .primitives
        .iter()
        .flat_map(|p| &p.positions)
        .map(|q| (rot * (Vec3::from(*q) - skull)).x)
        .fold(0.0f32, f32::max);
    let centre = Vec3::new(wall - 0.008 - back, hook.y - 0.07, hook.z);
    Transform::from_translation(centre - rot * skull).with_rotation(rot)
}

/// The inside of a single-sided shell: the same triangles turned (their
/// front faces and normals inwards), a hair inside it.
pub fn inner_lining(p: &Primitive) -> Primitive {
    let normals: Vec<[f32; 3]> = p.normals.iter().map(|n| [-n[0], -n[1], -n[2]]).collect();
    let positions = p
        .positions
        .iter()
        .zip(&normals)
        .map(|(q, n)| {
            [
                q[0] + n[0] * 0.001,
                q[1] + n[1] * 0.001,
                q[2] + n[2] * 0.001,
            ]
        })
        .collect();
    let indices = p
        .indices
        .chunks(3)
        .flat_map(|t| [t[0], t[2], t[1]])
        .collect();
    Primitive {
        material: p.material,
        positions,
        normals,
        indices,
        ..Default::default()
    }
}

/// A mesh of `p` with positions and normals only.
fn plain_mesh(p: &Primitive) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.positions.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
    m.insert_indices(Indices::U32(p.indices.clone()));
    m
}

/// The hook's peg and the helmet on it (with the kit it shows).
type HookState = (Option<Entity>, Option<(Vec<String>, Entity)>);

/// The helmet hanging on its hook (tests).
#[derive(Component)]
pub struct HelmetOnHook;

/// Keeps the mirror camera at the reflection of the workshop camera.
fn place_mirror(
    view: Res<WorkshopView>,
    scene: Res<Scene>,
    mut cams: Query<(&mut Transform, &mut Projection, &mut Camera)>,
) {
    let Some(cam) = scene.mirror_cam else { return };
    let layout = ocw_assets::workshop::layout(view.bikes.len());
    let (glass, half) = layout.mirror_glass();
    let Ok((mut t, mut p, mut c)) = cams.get_mut(cam) else {
        return;
    };
    let Some(m) = mirror_cam(view.eye, glass, half) else {
        c.is_active = false;
        return;
    };
    // Only while the corner is in the picture.
    c.is_active = view.rider.is_some() || view.eye.distance(glass) < 5.0;
    *t = Transform::from_translation(m.eye).looking_to(Vec3::NEG_X, Vec3::Y);
    if let Projection::Perspective(pp) = &mut *p {
        pp.fov = m.fov;
        pp.near = m.near;
    }
    c.sub_camera_view = Some(m.sub);
}

const FOCUS: Color = Color::srgb(1.0, 0.78, 0.24);

fn spawn_picker(
    commands: &mut Commands,
    panel: Entity,
    p: &PickerView,
    aspect: f32,
    thumbs: &mut crate::thumbs::Thumbnails,
    images: &mut Assets<Image>,
) {
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
    // Tiles: the thumbnails' images (kept, or queued for rendering).
    let tile_images: Vec<Option<Handle<Image>>> = p
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let shown = p
                .tiles
                .is_some_and(|t| i >= t.first && i < t.first + t.shown);
            r.thumb
                .as_ref()
                .filter(|_| shown)
                .map(|k| thumbs.image(k, images))
        })
        .collect();
    commands.entity(panel).with_children(|c| {
        c.spawn((
            Text::new(p.title.clone()),
            font(4.2),
            TextColor(Color::WHITE),
        ));
        if let Some(s) = &p.subtitle {
            c.spawn((
                Text::new(s.clone()),
                font(2.0),
                TextColor(Color::srgb(0.75, 0.9, 0.8)),
            ));
        }
        if let Some(s) = &p.swatches {
            let (_, _, inner_w) = panel_room(p, aspect);
            spawn_swatches(c, s, inner_w);
        }
        if let Some(t) = p.tiles {
            spawn_tiles(c, p, t, &tile_images);
        }
        let list_start = if p.tiles.is_some() {
            p.rows.len()
        } else {
            start
        };
        for (i, r) in p.rows.iter().enumerate().skip(list_start).take(SHOWN) {
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
        // Under the tiles the card is fitted to the room left in the panel
        // (bug 2026-11-17: the stand's card ran out at the panel's bottom).
        use panel_layout::*;
        let fitted = fitted_card(p, aspect);
        // No room for any line: no empty card under the tiles.
        if fitted.is_empty() {
            return;
        }
        c.spawn((
            Node {
                margin: UiRect::top(vh(CARD_MARGIN)),
                padding: UiRect::all(vh(CARD_PAD)),
                flex_direction: FlexDirection::Column,
                row_gap: vh(CARD_GAP),
                border_radius: BorderRadius::all(vh(0.8)),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
        ))
        .with_children(|d| {
            for (k, l) in fitted.iter().enumerate() {
                d.spawn((
                    Text::new(l.clone()),
                    font(if k == 0 && p.tiles.is_none() {
                        CARD_HEAD
                    } else {
                        CARD_TEXT
                    }),
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
    });
}

/// The picker panel's layout (vh): its top and bottom margins, padding,
/// border, width (vw) and the gap between its children; the fonts and
/// spacing of the tiles and of the card. [`fitted_card`] estimates the
/// panel's content with them.
mod panel_layout {
    pub const TOP: f32 = 8.5;
    pub const BOTTOM: f32 = 8.0;
    pub const PAD_X: f32 = 2.0;
    pub const PAD_Y: f32 = 1.6;
    pub const BORDER: f32 = 0.3;
    pub const WIDTH_VW: f32 = 40.0;
    pub const GAP: f32 = 0.6;
    pub const TITLE: f32 = 4.2;
    pub const SUBTITLE: f32 = 2.0;
    pub const MORE: f32 = 1.8;
    pub const GRID_GAP: f32 = 0.8;
    pub const TILE_PAD: f32 = 0.5;
    pub const TILE_BORDER: f32 = 0.35;
    pub const TILE_GAP: f32 = 0.2;
    pub const TILE_NAME: f32 = 1.75;
    pub const TILE_GAIN: f32 = 1.5;
    pub const TILE_FOOT: f32 = 1.6;
    pub const CARD_MARGIN: f32 = 1.0;
    pub const CARD_PAD: f32 = 1.2;
    pub const CARD_GAP: f32 = 0.4;
    pub const CARD_HEAD: f32 = 2.6;
    pub const CARD_TEXT: f32 = 2.2;
    /// The swatch row (4N Q30 (a)): its label, the swatches' gap, their
    /// largest and smallest size (smaller rows wrap) and the outline.
    pub const SWATCH_LABEL: f32 = 1.9;
    pub const SWATCH_GAP: f32 = 0.6;
    pub const SWATCH_MAX: f32 = 5.0;
    pub const SWATCH_MIN: f32 = 3.0;
    pub const SWATCH_OUTLINE: f32 = 0.45;
    /// Bevy's line height (of the font size) and the UI font's advance.
    pub const LINE: f32 = 1.2;
    pub const ADVANCE: f32 = 0.6;
}

/// Lines `text` takes in `width` (vh) at font `font` (vh): words wrapped
/// greedily (the UI font is monospaced), a too long word on its own.
pub fn wrapped_lines(text: &str, width: f32, font: f32) -> usize {
    let chars = ((width / (font * panel_layout::ADVANCE)).floor() as usize).max(1);
    let mut lines = 1;
    let mut used = 0;
    for word in text.split_whitespace() {
        let n = word.chars().count();
        if used > 0 && used + 1 + n > chars {
            lines += 1;
            used = 0;
        }
        used += if used > 0 { 1 + n } else { n };
        while used > chars {
            lines += 1;
            used -= chars;
        }
    }
    lines
}

/// The height (vh) of `lines` lines at `font`.
fn text_vh(lines: usize, font: f32) -> f32 {
    lines as f32 * font * panel_layout::LINE
}

/// The picker panel's height above the card (vh) in a window of `aspect`:
/// the title, the line under it and, with tiles, the grid with its "More"
/// lines – as the panel lays them out – and the panel's inner height and
/// width.
pub fn panel_room(p: &PickerView, aspect: f32) -> (f32, f32, f32) {
    use panel_layout::*;
    let inner_h = 100.0 - TOP - BOTTOM - 2.0 * (PAD_Y + BORDER);
    let inner_w = WIDTH_VW * aspect - 2.0 * (PAD_X + BORDER);
    let mut used = text_vh(wrapped_lines(&p.title, inner_w, TITLE), TITLE);
    if let Some(sub) = &p.subtitle {
        used += GAP + text_vh(wrapped_lines(sub, inner_w, SUBTITLE), SUBTITLE);
    }
    if let Some(s) = &p.swatches {
        let (size, rows) = swatch_layout(s.colours.len(), inner_w);
        used += GAP
            + text_vh(wrapped_lines(&s.label, inner_w, SWATCH_LABEL), SWATCH_LABEL)
            + GAP
            + rows as f32 * size
            + SWATCH_GAP * rows.saturating_sub(1) as f32;
    }
    if let Some(t) = p.tiles {
        if t.first > 0 {
            used += GAP + text_vh(1, MORE);
        }
        let cols = t.cols.max(1) as f32;
        let tile_w = inner_w * (100.0 - 2.5 * (cols - 1.0)) / cols / 100.0;
        let text_w = tile_w - 2.0 * (TILE_PAD + TILE_BORDER);
        let tile_h = |r: &PickerRowView| {
            let pic = text_w / tile_picture_aspect(r);
            let foot = [r.weight.as_str(), r.right.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let mut h = 2.0 * (TILE_PAD + TILE_BORDER)
                + pic
                + TILE_GAP
                + text_vh(wrapped_lines(&r.left, text_w, TILE_NAME), TILE_NAME)
                + TILE_GAP
                + text_vh(wrapped_lines(&foot, text_w, TILE_FOOT), TILE_FOOT);
            if !r.gain.is_empty() {
                h += TILE_GAP + text_vh(wrapped_lines(&r.gain, text_w, TILE_GAIN), TILE_GAIN);
            }
            h
        };
        let shown: Vec<&PickerRowView> = p.rows.iter().skip(t.first).take(t.shown).collect();
        let rows: Vec<f32> = shown
            .chunks(t.cols.max(1))
            .map(|row| row.iter().map(|r| tile_h(r)).fold(0.0, f32::max))
            .collect();
        if !rows.is_empty() {
            used += GAP + rows.iter().sum::<f32>() + GRID_GAP * (rows.len() - 1) as f32;
        }
        if t.first + t.shown < p.rows.len() {
            used += GAP + text_vh(1, MORE);
        }
    }
    (used, inner_h, inner_w)
}

/// The card as shown (the compare card while a part is pinned; bug
/// 2026-11-17: under the tiles the card ran out at the panel's bottom):
/// with tiles, the card's lines – in their order of importance, the app
/// puts the benefit and the bike first – that fit the room left under the
/// grid, all in the card's text size; a line that does not fit is left
/// out (a shorter one after it may still fit). Under a list the card is
/// shown whole.
pub fn fitted_card(p: &PickerView, aspect: f32) -> Vec<String> {
    use panel_layout::*;
    let card = p.compare.as_ref().unwrap_or(&p.detail);
    if p.tiles.is_none() {
        return card.clone();
    }
    let (used, inner_h, inner_w) = panel_room(p, aspect);
    let text_w = inner_w - 2.0 * CARD_PAD;
    // Half a line to spare: the estimate never cuts the card.
    let mut room =
        inner_h - used - GAP - CARD_MARGIN - 2.0 * CARD_PAD - text_vh(1, CARD_TEXT) * 0.5;
    let mut out = Vec::new();
    for line in card {
        // Under the tiles every line in the card's text size (the name is
        // on the focused tile).
        let font = CARD_TEXT;
        let gap = if out.is_empty() { 0.0 } else { CARD_GAP };
        let h = gap + text_vh(wrapped_lines(line, text_w, font), font);
        if h <= room {
            room -= h;
            out.push(line.clone());
        }
    }
    out
}

/// Width over height of a tile's picture (spec 4G *Tiles*): the
/// thumbnail's own aspect – the part's 2 : 1 ([`crate::thumbs::THUMB_SIZE`]),
/// the rider's square (4N) –, so the tile shows the whole rendering,
/// neither cropped nor stretched; a swatch is square, a tile without a
/// picture keeps the parts' room.
pub fn tile_picture_aspect(r: &PickerRowView) -> f32 {
    match (&r.thumb, r.swatch) {
        (Some(k), _) => k.aspect(),
        (None, Some(_)) => 1.0,
        (None, None) => crate::thumbs::THUMB_SIZE.x as f32 / crate::thumbs::THUMB_SIZE.y as f32,
    }
}

/// The swatch row's layout in a panel `inner_w` (vh) wide: the swatches'
/// size (vh, square) and the rows they take – one row of up to
/// [`panel_layout::SWATCH_MAX`] each while it fits, else rows of the
/// smallest size that still takes a finger or a click.
pub fn swatch_layout(n: usize, inner_w: f32) -> (f32, usize) {
    use panel_layout::*;
    if n == 0 {
        return (0.0, 0);
    }
    let one_row = (inner_w - SWATCH_GAP * (n - 1) as f32) / n as f32;
    if one_row >= SWATCH_MIN {
        return (one_row.min(SWATCH_MAX), 1);
    }
    let per_row = (((inner_w + SWATCH_GAP) / (SWATCH_MIN + SWATCH_GAP)).floor() as usize).max(1);
    (SWATCH_MIN, n.div_ceil(per_row))
}

/// The swatch row above a kit picker's tiles (4N Q30 (a)): its label,
/// then each colour as a square swatch, the selected one outlined (in the
/// focus colour while the row has the focus); hovering selects, a click
/// or a tap applies.
fn spawn_swatches(c: &mut ChildSpawnerCommands, s: &SwatchRowView, inner_w: f32) {
    use panel_layout::*;
    let (size, _) = swatch_layout(s.colours.len(), inner_w);
    c.spawn((
        Text::new(s.label.clone()),
        TextFont {
            font_size: FontSize::Vh(SWATCH_LABEL),
            ..default()
        },
        TextColor(if s.focused {
            FOCUS
        } else {
            Color::srgb(0.85, 0.87, 0.9)
        }),
    ));
    c.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        column_gap: vh(SWATCH_GAP),
        row_gap: vh(SWATCH_GAP),
        ..default()
    })
    .with_children(|row| {
        for (i, colour) in s.colours.iter().enumerate() {
            let [r, g, b] = colour.channels();
            let selected = i == s.selected;
            row.spawn((
                Node {
                    width: vh(size),
                    height: vh(size),
                    border: UiRect::all(vh(SWATCH_OUTLINE)),
                    border_radius: BorderRadius::all(vh(0.6)),
                    padding: UiRect::all(vh(SWATCH_OUTLINE * 0.6)),
                    ..default()
                },
                BorderColor::all(match (selected, s.focused) {
                    (true, true) => FOCUS,
                    (true, false) => Color::WHITE,
                    _ => Color::srgba(1.0, 1.0, 1.0, 0.18),
                }),
                BackgroundColor(Color::NONE),
            ))
            .with_children(|sw| {
                let mut inner = sw.spawn((
                    Node {
                        width: percent(100),
                        height: percent(100),
                        border_radius: BorderRadius::all(vh(0.35)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgb_u8(r, g, b)),
                ));
                // The standard colour's mark, light on dark colours.
                if s.standard == Some(i) {
                    let [lr, lg, lb] = [r, g, b].map(f32::from);
                    let light = 0.299 * lr + 0.587 * lg + 0.114 * lb < 128.0;
                    inner.with_children(|m| {
                        m.spawn((
                            Text::new("std"),
                            TextFont {
                                font_size: FontSize::Vh(size * 0.3),
                                ..default()
                            },
                            TextColor(if light {
                                Color::srgb(0.9, 0.9, 0.9)
                            } else {
                                Color::srgb(0.15, 0.15, 0.17)
                            }),
                        ));
                    });
                }
            })
            .observe(
                move |_: On<PointerOver>, mut ptr: ResMut<WorkshopPointer>| {
                    ptr.0.push(WorkshopPointerEvent::Swatch(i));
                },
            )
            .observe(
                move |click: On<PointerClick>, mut ptr: ResMut<WorkshopPointer>| {
                    if click.button == PointerButton::Primary {
                        ptr.0.push(WorkshopPointerEvent::SwatchClick(i));
                    }
                },
            );
        }
    });
}

/// The picker's tiles (spec 4B *Tiles*): the shown rows of the grid, each
/// a thumbnail of the part, its name, its gain and weight against the
/// slot's base part and its badge (equipped, owned, price, lock).
fn spawn_tiles(
    c: &mut ChildSpawnerCommands,
    p: &PickerView,
    t: TilesView,
    images: &[Option<Handle<Image>>],
) {
    let font = |vh: f32| TextFont {
        font_size: FontSize::Vh(vh),
        ..default()
    };
    let dim = Color::srgb(0.7, 0.72, 0.76);
    if t.first > 0 {
        c.spawn((Text::new("More above"), font(1.8), TextColor(dim)));
    }
    let cols = t.cols.max(1) as f32;
    c.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        column_gap: vh(0.8),
        row_gap: vh(0.8),
        ..default()
    })
    .with_children(|g| {
        // While the swatch row above has the focus no tile shows it.
        let on_swatches = p.swatches.as_ref().is_some_and(|s| s.focused);
        for (i, r) in p.rows.iter().enumerate().skip(t.first).take(t.shown) {
            let focused = i == p.focus && !on_swatches;
            let gain_colour = if r.gain.starts_with('+') {
                Color::srgb(0.6, 0.92, 0.65)
            } else if r.gain.starts_with('−') {
                Color::srgb(0.95, 0.65, 0.6)
            } else {
                Color::srgb(0.85, 0.87, 0.9)
            };
            g.spawn((
                Node {
                    width: percent((100.0 - 2.5 * (cols - 1.0)) / cols),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(vh(0.5)),
                    border: UiRect::all(vh(0.35)),
                    border_radius: BorderRadius::all(vh(0.8)),
                    row_gap: vh(0.2),
                    min_height: px(48),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(if focused {
                    Color::srgba(1.0, 0.78, 0.24, 0.22)
                } else {
                    Color::srgba(1.0, 1.0, 1.0, 0.06)
                }),
                BorderColor::all(if focused { FOCUS } else { Color::NONE }),
            ))
            .with_children(|tile| {
                let mut pic = tile.spawn((
                    Node {
                        width: percent(100),
                        // The whole thumbnail, neither cropped nor
                        // stretched: its own aspect (tile_picture_aspect).
                        aspect_ratio: Some(tile_picture_aspect(r)),
                        border_radius: BorderRadius::all(vh(0.5)),
                        ..default()
                    },
                    // Darker than the rendered background until it fills in.
                    BackgroundColor(Color::srgb(0.16, 0.16, 0.17)),
                ));
                if let Some(Some(img)) = images.get(i) {
                    pic.insert(ImageNode::new(img.clone()));
                } else if let Some(c) = r.swatch {
                    let [red, g, b] = c.channels();
                    pic.insert(BackgroundColor(Color::srgb_u8(red, g, b)));
                }
                tile.spawn((
                    Text::new(r.left.clone()),
                    font(1.75),
                    TextColor(Color::WHITE),
                ));
                if !r.gain.is_empty() {
                    tile.spawn((Text::new(r.gain.clone()), font(1.5), TextColor(gain_colour)));
                }
                let mut foot: Vec<&str> = Vec::new();
                if !r.weight.is_empty() {
                    foot.push(&r.weight);
                }
                if !r.right.is_empty() {
                    foot.push(&r.right);
                }
                tile.spawn((
                    Text::new(foot.join(" · ")),
                    font(1.6),
                    TextColor(if r.locked {
                        Color::srgb(0.75, 0.55, 0.55)
                    } else {
                        Color::srgb(1.0, 0.6, 0.7)
                    }),
                ));
            })
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
    });
    if t.first + t.shown < p.rows.len() {
        c.spawn((Text::new("More below"), font(1.8), TextColor(dim)));
    }
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

    /// Regression (2026-11-17, the helmet on the hook read faintly, "inside
    /// out"): every helmet hangs upright, its front towards the room, the
    /// peg inside the shell, the shell against the wall and not in it; the
    /// shell's liner faces inwards, so the open face shows the inside, not
    /// the wall through it.
    #[test]
    fn the_helmet_hangs_on_its_hook_upright_facing_the_room() {
        let model = ocw_assets::shipped::rider_model();
        let skull = ocw_assets::rider::skull_centre();
        for bikes in [1, 6, 24] {
            let layout = ocw_assets::workshop::layout(bikes);
            let hook = layout.helmet_hook();
            let wall = layout.bounds.1.x;
            for h in ocw_assets::helmet::Helmet::ALL {
                let i = model.node(h.node()).expect("helmet node");
                let t = hook_helmet_transform(model, i, hook, wall);
                let up = t.rotation * Vec3::Y;
                let front = t.rotation * Vec3::NEG_Z;
                assert!(up.y > 0.9, "{h:?} upright: {up}");
                assert!(front.x < -0.9, "{h:?} facing the room: {front}");
                let pts: Vec<Vec3> = model.nodes[i]
                    .primitives
                    .iter()
                    .flat_map(|p| &p.positions)
                    .map(|q| t.transform_point(Vec3::from(*q)))
                    .collect();
                let max = pts.iter().fold(Vec3::MIN, |a, b| a.max(*b));
                let min = pts.iter().fold(Vec3::MAX, |a, b| a.min(*b));
                assert!(
                    max.x < wall && max.x > wall - 0.03,
                    "{h:?} against the wall: {max}"
                );
                assert!(min.x > wall - 0.6, "{h:?}");
                // The peg (from the wall to the hook) inside the shell's
                // height and width.
                assert!(min.y < hook.y && hook.y < max.y, "{h:?} on the peg");
                assert!(min.z < hook.z && hook.z < max.z, "{h:?}");
                // The skull's centre (where a head would be) under the peg.
                let head = model.node(ocw_assets::pose::nodes::HEAD).unwrap();
                let c = t.transform_point(skull - Vec3::from(model.world(head).translation));
                assert!((hook.y - c.y - 0.07).abs() < 1e-4);
            }
        }
        // The liner: the shell's triangles turned to face its middle.
        let i = model.node(ocw_assets::helmet::Helmet::Road.node()).unwrap();
        let head = model.node(ocw_assets::pose::nodes::HEAD).unwrap();
        let centre = skull - Vec3::from(model.world(head).translation);
        for p in &model.nodes[i].primitives {
            if model.materials[p.material].name != "helmet" {
                continue;
            }
            let facing = |q: &Primitive| {
                q.indices
                    .chunks(3)
                    .filter(|t| {
                        let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(q.positions[t[k] as usize]));
                        (b - a).cross(c - a).dot((a + b + c) / 3.0 - centre) > 0.0
                    })
                    .count() as f32
                    / (q.indices.len() / 3) as f32
            };
            let liner = inner_lining(p);
            assert!(facing(p) > 0.7, "the shell faces out");
            assert!(facing(&liner) < 0.3, "the liner faces in");
            assert_eq!(liner.indices.len(), p.indices.len());
        }
    }

    /// Six groupset tiles and a long card, as in the picture of the bug.
    fn groupset_tiles(detail: Vec<String>, shown: usize) -> PickerView {
        let names = [
            "Stock road groupset",
            "Endurance 11-speed mechanical groupset",
            "Endurance 12-speed mechanical groupset",
            "Endurance 12-speed electronic groupset",
            "Race 12-speed electronic groupset, upper tier",
            "Race 12-speed electronic groupset",
        ];
        PickerView {
            title: "Groupset – Road - standard".into(),
            subtitle: Some(
                "This bike: ±0.0 % flat · ±0.0 % climb vs stock · 8.00 kg (stock weight)".into(),
            ),
            rows: names
                .iter()
                .map(|n| PickerRowView {
                    left: n.to_string(),
                    right: "Upper · 2,549 ♥".into(),
                    thumb: Some(crate::thumbs::ThumbKey::Part(crate::thumbs::PartThumb {
                        profile: ProfileId::Road,
                        slot: "groupset".into(),
                        model: "stock".into(),
                        paint: Rgb(0x808080),
                    })),
                    gain: "+0.1 % flat · +0.8 % climb".into(),
                    weight: "−1,500 g".into(),
                    ..Default::default()
                })
                .collect(),
            focus: 4,
            detail,
            tiles: Some(TilesView {
                cols: 3,
                first: 0,
                shown,
            }),
            ..Default::default()
        }
    }

    /// Regression (2026-11-17, the bike stand's card ran out at the
    /// panel's bottom under two rows of tiles): the panel's estimate
    /// matches the picture of the bug (2560 × 1440: the grid ends about
    /// 65 vh into the panel), and in 16:9, 16:10, 4:3 and 21:9 windows the
    /// card shows only what fits – a part of its lines, in order, never
    /// nothing; a short card and a card under a list are shown whole.
    #[test]
    fn the_stand_card_fits_under_the_tiles() {
        use panel_layout::*;
        let long: Vec<String> = [
            "Benefit: −500 g: +0.8 % at 6 %, +0.1 % flat",
            "Bike: 7.50 kg · +0.1 % flat, +0.8 % at 6 % vs stock",
            "Upper tier · 2,600 g · 2,549 ♥",
            "Against the base part: +0.1 % flat · +0.8 % climb · −500 g",
            "Effect: −500 g · drivetrain +0.6 %",
            "Fitted with: Race crankset · Race 12-speed cassette · Race chain (in the price and weight; the parts they replace go to your spare parts)",
            "Counts in free ride and Open races; cosmetic in Profile / Equal races (the default)",
        ]
        .map(String::from)
        .to_vec();
        let v = groupset_tiles(long.clone(), 6);
        let (used, inner_h, _) = panel_room(&v, 16.0 / 9.0);
        assert!((60.0..72.0).contains(&used), "{used} vh above the card");
        assert!(
            used + 2.0 * CARD_PAD + text_vh(long.len(), CARD_TEXT) > inner_h,
            "it ran out"
        );
        for aspect in [16.0 / 9.0, 16.0 / 10.0, 4.0 / 3.0, 21.0 / 9.0] {
            let shown = fitted_card(&v, aspect);
            assert!(!shown.is_empty(), "{aspect}");
            let mut rest = long.iter();
            assert!(
                shown.iter().all(|l| rest.any(|d| d == l)),
                "{aspect}: {shown:?}"
            );
            let (used, inner_h, inner_w) = panel_room(&v, aspect);
            let w = inner_w - 2.0 * CARD_PAD;
            let card: f32 = shown
                .iter()
                .map(|l| text_vh(wrapped_lines(l, w, CARD_TEXT), CARD_TEXT))
                .sum::<f32>()
                + CARD_GAP * (shown.len() - 1) as f32;
            let total = used + GAP + CARD_MARGIN + 2.0 * CARD_PAD + card;
            assert!(total <= inner_h, "{aspect}: {total} of {inner_h} vh");
        }
        // The benefit comes first and is kept at 16:9.
        assert_eq!(fitted_card(&v, 16.0 / 9.0)[0], long[0]);
        // One row of tiles and a short card: whole.
        let short = vec!["Benefit: +0.1 % flat".to_string(), "Bike: 8.00 kg".into()];
        assert_eq!(
            fitted_card(&groupset_tiles(short.clone(), 3), 16.0 / 9.0),
            short
        );
        // Under a list the card is whole.
        let mut list = groupset_tiles(long.clone(), 6);
        list.tiles = None;
        assert_eq!(fitted_card(&list, 4.0 / 3.0), long);
        // The word wrap of the estimate.
        assert_eq!(wrapped_lines("aaa bbb ccc", 7.0 * 2.0 * 0.6 + 0.01, 2.0), 2);
        assert_eq!(wrapped_lines("", 10.0, 2.0), 1);
        assert_eq!(wrapped_lines("abcdefghij", 4.0 * 0.6 + 0.01, 1.0), 3);
    }

    /// The kit pickers' swatch row (4N Q30 (a)): the thirteen colours in
    /// one row in every window from 4:3 to 21:9, each at most
    /// `SWATCH_MAX`; a narrow panel wraps them at the smallest size; the
    /// panel's estimate counts the row, and the card still fits under
    /// two rows of rider tiles.
    #[test]
    fn the_swatch_row_fits_above_the_tiles() {
        use panel_layout::*;
        for aspect in [4.0 / 3.0, 16.0 / 10.0, 16.0 / 9.0, 21.0 / 9.0] {
            let inner_w = WIDTH_VW * aspect - 2.0 * (PAD_X + BORDER);
            let (size, rows) = swatch_layout(13, inner_w);
            assert_eq!(rows, 1, "{aspect}");
            assert!(
                (SWATCH_MIN..=SWATCH_MAX).contains(&size),
                "{aspect}: {size}"
            );
            assert!(
                13.0 * size + 12.0 * SWATCH_GAP <= inner_w + 1e-3,
                "{aspect}"
            );
        }
        // A phone held upright: rows of the smallest swatches.
        let (size, rows) = swatch_layout(13, 30.0);
        assert_eq!(size, SWATCH_MIN);
        assert!(rows > 1);
        let per_row = 13usize.div_ceil(rows);
        assert!(per_row as f32 * SWATCH_MIN + (per_row - 1) as f32 * SWATCH_GAP <= 30.0);
        assert_eq!(swatch_layout(0, 30.0), (0.0, 0));
        // The estimate: the row adds its label and its swatches.
        // Two rows of square rider tiles, as the gloves' picker shows them.
        let mut v = PickerView {
            title: "Gloves – your kit".into(),
            rows: (0..7)
                .map(|i| PickerRowView {
                    left: format!("Race gloves {i}"),
                    right: "1,240 ♥".into(),
                    swatch: Some(Rgb(0x1E1E22)),
                    ..Default::default()
                })
                .collect(),
            detail: vec!["Free – looks only".into()],
            tiles: Some(TilesView {
                cols: 3,
                first: 0,
                shown: 6,
            }),
            ..Default::default()
        };
        let (before, _, _) = panel_room(&v, 16.0 / 9.0);
        v.swatches = Some(SwatchRowView {
            label: "Colour of the gloves: standard (black)".into(),
            colours: vec![Rgb(0x1E1E22); 13],
            selected: 0,
            focused: true,
            standard: Some(0),
        });
        let (after, inner_h, inner_w) = panel_room(&v, 16.0 / 9.0);
        let (size, _) = swatch_layout(13, inner_w);
        let label = text_vh(1, SWATCH_LABEL);
        assert!((after - before - (2.0 * GAP + label + size)).abs() < 1e-3);
        assert!(after < inner_h, "{after} of {inner_h}");
        assert_eq!(
            fitted_card(&v, 16.0 / 9.0),
            vec!["Free – looks only".to_string()]
        );
    }

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

    /// The planar reflection (spec 4C): what the mirror camera sees of a
    /// room point at a picture position is what the eye sees of the point
    /// mirrored at the glass, at the glass point mapped to the same
    /// position; the near plane lies on the glass; the sub view is the
    /// glass rectangle.
    #[test]
    fn the_mirror_camera_is_the_reflection_of_the_eye() {
        let layout = ocw_assets::workshop::layout(6);
        let (glass, half) = layout.mirror_glass();
        let eye = layout.dress_view(false).eye;
        let m = mirror_cam(eye, glass, half).expect("in front of the mirror");
        assert!((m.eye.x - glass.x - (glass.x - eye.x)).abs() < 1e-5);
        assert_eq!((m.eye.y, m.eye.z), (eye.y, eye.z));
        assert!((m.near - (glass.x - eye.x)).abs() < 0.01);
        assert_eq!(m.sub.size, UVec2::new(860, 1760));
        let full = m.sub.full_size.as_vec2();
        // A room point (the rider's head) and where each camera sees it.
        let q = layout.rider_hotspots(1.0)[0];
        // The mirror camera: through the plane, then into the sub view.
        let t = (glass.x - m.eye.x) / (q.x - m.eye.x);
        let hit = m.eye + (q - m.eye) * t;
        let d = glass.x - eye.x;
        let hh = (m.fov * 0.5).tan() * d;
        let hw = hh * full.x / full.y;
        let px = Vec2::new(eye.z - hit.z + hw, eye.y + hh - hit.y) * MIRROR_GRID;
        let uv_cam = (px - m.sub.offset) / m.sub.size.as_vec2();
        // The eye: the mirrored point, through the glass.
        let q2 = Vec3::new(2.0 * glass.x - q.x, q.y, q.z);
        let t = (glass.x - eye.x) / (q2.x - eye.x);
        let seen = eye + (q2 - eye) * t;
        let uv_glass = mirror_uv(glass, half, seen);
        assert!(uv_cam.distance(uv_glass) < 2e-3, "{uv_cam} vs {uv_glass}");
        assert!((0.0..=1.0).contains(&uv_glass.x) && (0.0..=1.0).contains(&uv_glass.y));
        // The glass corners map to the picture's corners.
        let top_left = mirror_uv(glass, half, glass + Vec3::new(0.0, half.y, half.x));
        assert!(top_left.distance(Vec2::ZERO) < 1e-5, "{top_left}");
        let bottom_right = mirror_uv(glass, half, glass + Vec3::new(0.0, -half.y, -half.x));
        assert!(bottom_right.distance(Vec2::ONE) < 1e-5, "{bottom_right}");
        // Behind the glass: no mirror camera.
        assert!(mirror_cam(glass + Vec3::X, glass, half).is_none());
    }

    /// *Try on the bike* (spec 4C): on the stand bike as built (with its
    /// parts), riders of every height sit on the saddle at their height and
    /// pedal at the slow try-on cadence with the feet on the pedals.
    #[test]
    fn the_try_on_pose_keeps_the_feet_on_the_pedals() {
        use ocw_assets::pose::{BikeRig, BodyFit, PoseParams, nodes, pose};
        let models = BikeModels::default();
        let rig = ocw_assets::shipped::rider_rig();
        let model = ocw_assets::shipped::rider_model();
        for (profile, variants) in [
            (ProfileId::Road, vec!["wheel-carbon-50".to_string()]),
            (ProfileId::TimeTrial, vec![]),
            (ProfileId::Mountain, vec![]),
        ] {
            let built = models.get(profile, &variants);
            let bike = BikeRig::from_model(&built.model).unwrap();
            for cm in [158.0f32, 178.0, 196.0] {
                let body = BodyFit::new(&rig, &bike, cm / 178.0);
                let mut crank = 0.0;
                for _ in 0..12 {
                    crank = ocw_assets::anim::advance_crank(crank, TRY_ON_RPM, 0.1);
                    let p = pose(
                        &body.rider,
                        &body.bike,
                        &body.fit,
                        &PoseParams {
                            crank,
                            ..Default::default()
                        },
                    );
                    let mut posed = model.clone();
                    p.apply(&mut posed);
                    let pi = posed.node(nodes::PELVIS).unwrap();
                    posed.nodes[pi].translation = p.pelvis.0 / body.scale;
                    for side in 0..2 {
                        let cleat = posed.position(nodes::CLEAT[side]).unwrap() * body.scale;
                        assert!(
                            cleat.distance(body.bike.pedal(side, crank)) < 0.03,
                            "{profile:?} {cm} cm"
                        );
                    }
                }
                // 45 rpm for 1.2 s: 0.9 of a turn.
                assert!(
                    (crank - 0.9 * std::f32::consts::TAU).abs() < 1e-3,
                    "{crank}"
                );
            }
        }
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
