//! The Models workspace's 3D view (spec §14 Phase 16, 16B, stage E3 –
//! view only): one model of `artwork/3dmodels/` (or a `.glb` / `.gltf`
//! file) on its own render layer, seen by its own orbit camera under the
//! Phase 9 light model's presets; a ground plane, wireframe and normals as
//! line meshes, per-node and per-material highlighting, the level of
//! detail by the camera's distance or chosen, gizmos for the contract's
//! anchors, hotspots and collision proxies, and the try-on scenes (wear on
//! the rider, a part on a bike, the rider on a bike in the riding poses, a
//! plant or prop in a small scene with the seasons). Every model is drawn
//! from its file through the Phase 15 reader; nothing is edited (E4).
//!
//! The pure parts are in [`crate::models`]; the console has them as
//! `model open ID|FILE | show … | light … | lod … | select … | orbit … |
//! try … | info | list`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, Exposure, Viewport};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_assets::check::{self, Finding};
use ocw_assets::kinds::Kind;
use ocw_assets::library::ModelLibrary;
use ocw_assets::manifest::Entry;
use ocw_assets::model::Model;
use ocw_core::{BodyShape, Character, ProfileId};
use ocw_render::camera::FollowCamera;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

use crate::models::{self, Gizmo, GizmoKind, Level, LightPreset, LodMode, Orbit, RidePose, Stats};

/// The Models view's render layer (the renderer uses 0 – 9).
pub const MODEL_LAYER: usize = 11;

fn layer() -> RenderLayers {
    RenderLayers::layer(MODEL_LAYER)
}

/// What is highlighted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Highlight {
    #[default]
    None,
    /// A node of the open model and what is below it.
    Node(usize),
    /// Every mesh of a material of the open model.
    Material(String),
}

/// The seasons of the scene try-on (the plants' slots tinted as
/// `ocw_render::flora` does).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Season {
    Spring,
    #[default]
    Summer,
    Autumn,
    Winter,
}

impl Season {
    pub const ALL: [Self; 4] = [Self::Spring, Self::Summer, Self::Autumn, Self::Winter];

    pub fn name(self) -> &'static str {
        match self {
            Self::Spring => "spring",
            Self::Summer => "summer",
            Self::Autumn => "autumn",
            Self::Winter => "winter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.name() == s)
    }

    /// The plant look of the season.
    pub fn look(self) -> ocw_render::flora::PlantLook {
        let mut l = ocw_render::flora::PlantLook::SUMMER;
        match self {
            Self::Spring => {
                l.fresh = 1.0;
                l.bloom = 1.0;
            }
            Self::Summer => {}
            Self::Autumn => l.autumn = 1.0,
            Self::Winter => l.leaves = 0.0,
        }
        l
    }
}

/// The try-on of the open model (16B *Try it on*).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TryOn {
    /// The model alone.
    #[default]
    Off,
    /// A wearable on the rider of this build.
    Rider { shape: BodyShape, height_cm: u16 },
    /// A part at this bike's hotspot.
    Bike(ProfileId),
    /// The rider on a bike (a rider or a bike opened) in a pose.
    Ride {
        bike: ProfileId,
        pose: RidePose,
        shape: BodyShape,
        height_cm: u16,
        pedal: bool,
    },
    /// A plant or prop in a small scene.
    Scene(Season),
}

impl TryOn {
    /// The try-on a kind offers (`None`: none yet).
    pub fn default_for(kind: Kind, id: &str) -> Option<TryOn> {
        let ride = |bike| TryOn::Ride {
            bike,
            pose: RidePose::Hoods,
            shape: BodyShape::Normal,
            height_cm: ocw_core::character::DEFAULT_HEIGHT_CM,
            pedal: true,
        };
        match kind {
            Kind::Wearable => Some(TryOn::Rider {
                shape: BodyShape::Normal,
                height_cm: ocw_core::character::DEFAULT_HEIGHT_CM,
            }),
            Kind::Part => Some(TryOn::Bike(ProfileId::Road)),
            Kind::Rider => Some(ride(ProfileId::Road)),
            Kind::Bike => Some(ride(
                ProfileId::ALL
                    .into_iter()
                    .find(|p| ocw_assets::bike_id(*p) == id)
                    .unwrap_or(ProfileId::Road),
            )),
            Kind::Plant | Kind::Prop => Some(TryOn::Scene(Season::Summer)),
            Kind::Creature | Kind::Workshop | Kind::Vehicle => None,
        }
    }
}

/// The open model.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The library id, or the file's path.
    pub id: String,
    pub path: Option<PathBuf>,
    /// The manifest entry (a file outside the folder: one made from its
    /// `extras.ocw` or its folder, for the checks).
    pub entry: Entry,
    pub listed: bool,
    pub model: Arc<Model>,
    pub bytes: usize,
    pub findings: Vec<Finding>,
    pub stats: Stats,
    pub gizmos: Vec<Gizmo>,
}

/// The Models workspace's state.
#[derive(Resource)]
pub struct ModelView {
    pub lib: ModelLibrary,
    /// Where the library is (a folder or "embedded").
    pub lib_label: String,
    pub open: Option<Opened>,
    pub search: String,
    pub orbit: Orbit,
    pub turntable: bool,
    pub light: LightPreset,
    pub ground: bool,
    pub wireframe: bool,
    pub normals: bool,
    pub gizmos: bool,
    pub labels: bool,
    pub lod: LodMode,
    pub highlight: Highlight,
    pub tryon: TryOn,
    /// The scene must be built again.
    pub dirty: bool,
    pub style_dirty: bool,
    /// The level shown last frame and the camera's distance to the model.
    pub shown: Level,
    pub distance: f32,
    pub status: String,
    /// Crank angle of the ride try-on (rad).
    pub crank: f32,
    /// The unicorn's flight cycles ([`crate::models_cycle`]).
    pub cycle: crate::models_cycle::CyclePlayer,
    reload_timer: f32,
    /// The open model is a document of the edit stage (E4,
    /// [`crate::model_edit`]): a changed file is re-imported into it
    /// (`external_change`) instead of opened anew.
    pub editing: bool,
    pub external_change: bool,
    /// The selected node (the edit gizmo's), by index.
    pub selected: Option<usize>,
    /// Only transforms changed: the drawn nodes are moved, not rebuilt.
    pub retransform: bool,
    /// The edit gizmo: its mode, the handle under the pointer or dragged.
    pub gizmo_mode: crate::model_gizmo::GizmoMode,
    pub gizmo_hot: Option<usize>,
    pub gizmo_drag: Option<crate::model_gizmo::GizmoDrag>,
}

impl ModelView {
    pub fn new(lib: ModelLibrary, lib_label: String) -> Self {
        Self {
            lib,
            lib_label,
            open: None,
            search: String::new(),
            orbit: Orbit::default(),
            turntable: false,
            light: LightPreset::Day,
            ground: true,
            wireframe: false,
            normals: false,
            gizmos: true,
            labels: true,
            lod: LodMode::Auto,
            highlight: Highlight::None,
            tryon: TryOn::Off,
            dirty: true,
            style_dirty: true,
            shown: Level::Near,
            distance: 0.0,
            status: String::new(),
            crank: 0.0,
            cycle: Default::default(),
            reload_timer: 0.0,
            editing: false,
            external_change: false,
            selected: None,
            retransform: false,
            gizmo_mode: crate::model_gizmo::GizmoMode::Move,
            gizmo_hot: None,
            gizmo_drag: None,
        }
    }

    /// Shows an edited version of the open model (E4): its findings,
    /// stats and gizmos again; only moved nodes are moved, anything else
    /// is drawn anew.
    pub fn refresh(&mut self, model: Model, entry: Entry) {
        let Some(o) = &self.open else { return };
        let moved_only = o.entry == entry
            && o.model.shade_scale() == model.shade_scale()
            && o.model.materials == model.materials
            && o.model.nodes.len() == model.nodes.len()
            && o.model.nodes.iter().zip(&model.nodes).all(|(a, b)| {
                a.name == b.name
                    && a.parent == b.parent
                    && a.extras == b.extras
                    && a.primitives == b.primitives
            });
        // (The file's size changes with the geometry, not with a move.)
        let bytes = if moved_only {
            o.bytes
        } else {
            ocw_assets::glb::write(&model).len()
        };
        let next = opened(
            o.id.clone(),
            o.path.clone(),
            entry,
            o.listed,
            Arc::new(model),
            bytes,
        );
        self.open = Some(next);
        // A try-on that poses the open model is drawn anew.
        let posed = matches!(self.tryon, TryOn::Ride { .. } | TryOn::Rider { .. });
        if moved_only && !posed {
            self.retransform = true;
        } else {
            self.dirty = true;
        }
        if let Highlight::Node(i) = self.highlight
            && self.open.as_ref().is_some_and(|o| i >= o.model.nodes.len())
        {
            self.highlight = Highlight::None;
        }
        self.style_dirty = true;
    }

    /// The library: a folder (`--models`, `OCW_MODELS`, or
    /// `artwork/3dmodels` next to the working directory) or the embedded
    /// files.
    pub fn library(dir: Option<&Path>) -> (ModelLibrary, String) {
        let env = std::env::var_os("OCW_MODELS").map(PathBuf::from);
        let local = PathBuf::from("artwork/3dmodels");
        let pick = dir.map(Path::to_path_buf).or(env).or_else(|| {
            local
                .join(ocw_assets::manifest::MANIFEST)
                .is_file()
                .then_some(local)
        });
        if let Some(d) = pick {
            match ModelLibrary::folder(&d) {
                Ok(l) => return (l, d.display().to_string()),
                Err(e) => warn!(
                    "model folder {}: {e}; using the embedded models",
                    d.display()
                ),
            }
        }
        (ModelLibrary::embedded(), "embedded".into())
    }

    /// Opens a library id, or a `.glb` / `.gltf` file.
    pub fn open(&mut self, what: &str) -> Result<String, String> {
        let what = what.trim();
        let opened = if self.lib.entry(ocw_assets::kinds::id_of(what)).is_some() {
            open_id(&self.lib, ocw_assets::kinds::id_of(what))?
        } else {
            open_file(Path::new(what))?
        };
        let msg = format!(
            "{}: {} triangles, {} errors, {} warnings",
            opened.id,
            opened.stats.triangles,
            opened.findings.iter().filter(|f| f.is_error()).count(),
            opened.findings.iter().filter(|f| !f.is_error()).count()
        );
        self.tryon = TryOn::Off;
        self.highlight = Highlight::None;
        if let Some((lo, hi)) = opened.stats.bounds {
            self.orbit = Orbit::framing(lo, hi);
        }
        self.open = Some(opened);
        self.dirty = true;
        self.status = msg.clone();
        Ok(msg)
    }

    /// Frames the open model (or the try-on scene).
    pub fn frame(&mut self) {
        let Some(o) = &self.open else { return };
        let (lo, hi) = o
            .stats
            .bounds
            .unwrap_or((Vec3::splat(-0.5), Vec3::splat(0.5)));
        self.orbit = match &self.tryon {
            TryOn::Off | TryOn::Bike(_) => Orbit::framing(lo, hi),
            TryOn::Rider { height_cm, .. } => {
                let anchor = o.entry.anchor.as_deref().unwrap_or("head");
                let s = f32::from(*height_cm) / f32::from(ocw_core::character::DEFAULT_HEIGHT_CM);
                let rider = ocw_assets::shipped::rider_model();
                let at = rider.position(anchor).unwrap_or(Vec3::Y) * s + Vec3::Y * standing_lift(s);
                let mut orbit = Orbit::framing(at - Vec3::splat(0.35), at + Vec3::splat(0.35));
                orbit.yaw = 0.9;
                orbit
            }
            TryOn::Ride { .. } => {
                Orbit::framing(Vec3::new(-0.4, 0.0, -1.0), Vec3::new(0.4, 1.8, 1.0))
            }
            TryOn::Scene(_) => {
                // The copies stand up to about 3.3 sizes out (see rebuild).
                let r = (hi - lo).xz().length().max(0.3) * 3.3;
                Orbit {
                    pitch: 0.3,
                    ..Orbit::framing(Vec3::new(-r, 0.0, -r), Vec3::new(r, hi.y.max(1.0), r))
                }
            }
        };
    }

    pub fn set_tryon(&mut self, t: TryOn) {
        self.tryon = t;
        self.dirty = true;
        self.frame();
    }

    /// The level shown now.
    pub fn level(&self) -> Level {
        let Some(o) = &self.open else {
            return Level::Near;
        };
        models::shown_level(
            self.lod,
            self.distance,
            o.entry.kind,
            o.model.node("lod1").is_some(),
            o.model.node("billboard").is_some(),
        )
    }
}

fn open_id(lib: &ModelLibrary, id: &str) -> Result<Opened, String> {
    let entry = lib
        .entry(id)
        .cloned()
        .ok_or_else(|| format!("no model `{id}`"))?;
    let bytes = lib.bytes(id).map_err(|e| e.to_string())?;
    let model = Arc::new(ocw_assets::glb::read(&bytes).map_err(|e| format!("{id}: {e}"))?);
    Ok(opened(id.to_owned(), None, entry, true, model, bytes.len()))
}

/// An entry for a file outside the model folder: its `extras.ocw` kind
/// and id, else its folder's kind (`…/bikes/x.glb`), else a prop.
pub fn guess_entry(path: &Path, m: &Model) -> Entry {
    let ocw = m.ocw();
    let kind = ocw
        .and_then(|o| o.get("kind")?.as_str())
        .and_then(Kind::parse)
        .or_else(|| {
            let dir = path.parent()?.file_name()?.to_str()?;
            Kind::ALL.into_iter().find(|k| k.dir() == dir)
        })
        .unwrap_or(Kind::Prop);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let id = ocw
        .and_then(|o| o.get("id")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{}/{stem}", kind.dir()));
    let mut e = ocw_assets::export::entry(&format!("{id}.glb"), kind, "");
    e.generator = None;
    let get = |k: &str| ocw.and_then(|o| o.get(k)?.as_str().map(str::to_owned));
    e.profile = (kind == Kind::Bike).then(|| get("profile")).flatten();
    e.species = get("species");
    e.anchor = get("anchor");
    e
}

fn open_file(path: &Path) -> Result<Opened, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let model =
        Arc::new(ocw_assets::glb::read_file(path).map_err(|e| format!("{}: {e}", path.display()))?);
    let entry = guess_entry(path, &model);
    Ok(opened(
        path.display().to_string(),
        Some(path.to_path_buf()),
        entry,
        false,
        model,
        bytes.len(),
    ))
}

fn opened(
    id: String,
    path: Option<PathBuf>,
    entry: Entry,
    listed: bool,
    model: Arc<Model>,
    bytes: usize,
) -> Opened {
    let findings = if listed {
        check::check(&entry, &model, bytes)
    } else {
        // Outside the folder: the file's own checks (no manifest entry).
        check::check_model(&entry, &model, bytes)
    };
    Opened {
        stats: Stats::of(&model, Some(entry.kind), bytes),
        gizmos: models::gizmos(&model),
        id,
        path,
        entry,
        listed,
        model,
        bytes,
        findings,
    }
}

/// The feet on the ground: the standing rider's root this high (m, ×
/// scale).
fn standing_lift(scale: f32) -> f32 {
    let rig = ocw_assets::shipped::rider_rig();
    (rig.leg() - rig.cleat.y) * scale
}

/// Which model of the scene an entity belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The open model.
    Main,
    /// The rider or bike of a try-on.
    Rider,
    Bike,
    /// Copies of the open model in the scene try-on.
    Copy,
}

#[derive(Component)]
pub struct ModelCamera;

#[derive(Component)]
struct ModelSun;

#[derive(Component)]
struct ModelGround;

/// The scene's root (despawned on a rebuild).
#[derive(Component)]
struct SceneRoot;

/// A node of a spawned model.
#[derive(Component, Debug, Clone, Copy)]
pub struct ViewNode {
    pub part: Part,
    pub index: usize,
}

/// A mesh of a spawned model.
#[derive(Component, Debug, Clone)]
struct ViewPrim {
    part: Part,
    node: usize,
    material: String,
    level: Level,
    base: Handle<StandardMaterial>,
    /// Wire or normal lines (shown with their toggles), or the mesh.
    lines: Option<bool>,
}

/// The view's gizmos (on its layer only).
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct ModelGizmos;

/// The shared materials.
#[derive(Resource)]
struct ViewMats {
    highlight: Handle<StandardMaterial>,
    wire: Handle<StandardMaterial>,
    normal: Handle<StandardMaterial>,
    ground: Handle<StandardMaterial>,
    grass: Handle<StandardMaterial>,
    snow: Handle<StandardMaterial>,
}

/// The 3D view's rectangle from the panels (physical pixels).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct ViewportRect(pub Option<(UVec2, UVec2)>);

/// Whether the Models workspace is shown (set by the panels).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModelsActive(pub bool);

pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewportRect>()
            .init_resource::<ModelsActive>()
            .init_gizmo_group::<ModelGizmos>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    activate,
                    hot_reload,
                    crate::model_edit::sync,
                    rebuild,
                    retransform,
                    animate,
                    style,
                    camera_and_light,
                    draw_gizmos,
                )
                    .chain(),
            );
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("model")
                .about("OCW Studio's Models workspace: open ID|FILE | list [WORDS] | info | show wireframe|normals|ground|gizmos|labels|turntable on|off | light day|dusk|night|overcast | lod auto|near|lod1|billboard | select node NAME|material NAME|none | orbit YAW PITCH [DIST] | zoom DIST | frame | try off|rider SHAPE [CM]|bike PROFILE|ride PROFILE [POSE] [SHAPE] [CM]|pose POSE|scene SEASON")
                .arg(
                    Arg::new("words")
                        .num_args(1..)
                        .allow_hyphen_values(true)
                        .trailing_var_arg(true)
                        .action(ArgAction::Append),
                ),
            model_console,
        ));
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut store: ResMut<GizmoConfigStore>,
) {
    let (config, _) = store.config_mut::<ModelGizmos>();
    config.render_layers = layer();
    config.line.width = 2.0;
    config.depth_bias = -0.5;
    let rig = models::light_rig(LightPreset::Day);
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::srgb(
                rig.sky_srgb[0],
                rig.sky_srgb[1],
                rig.sky_srgb[2],
            )),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: models::FOV_Y,
            near: 0.01,
            ..default()
        }),
        Transform::from_xyz(2.0, 1.0, -2.0).looking_at(Vec3::Y * 0.5, Vec3::Y),
        AmbientLight {
            color: Color::WHITE,
            brightness: rig.ambient,
            ..default()
        },
        Exposure { ev100: rig.ev100 },
        ModelCamera,
        layer(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: rig.sun_lux,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_translation(rig.sun_dir).looking_at(Vec3::ZERO, Vec3::Y),
        ModelSun,
        layer(),
    ));
    let plain = |c: Color| StandardMaterial {
        base_color: c,
        perceptual_roughness: 0.95,
        ..default()
    };
    let mats = ViewMats {
        highlight: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.5, 0.05),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        }),
        wire: materials.add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.9, 1.0),
            unlit: true,
            ..default()
        }),
        normal: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.2, 0.9),
            unlit: true,
            ..default()
        }),
        ground: materials.add(plain(Color::srgb(0.42, 0.42, 0.40))),
        grass: materials.add(plain(Color::srgb(0.22, 0.34, 0.14))),
        snow: materials.add(plain(Color::srgb(0.86, 0.88, 0.92))),
    };
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 400.0))),
        MeshMaterial3d(mats.ground.clone()),
        Transform::default(),
        ModelGround,
        layer(),
    ));
    commands.insert_resource(mats);
}

/// The model camera draws while the Models workspace is shown; the world
/// camera the rest of the time.
fn activate(
    active: Res<ModelsActive>,
    rect: Res<ViewportRect>,
    mut models_cam: Query<&mut Camera, (With<ModelCamera>, Without<FollowCamera>)>,
    mut world_cam: Query<&mut Camera, (With<FollowCamera>, Without<ModelCamera>)>,
) {
    for mut c in &mut models_cam {
        if c.is_active != active.0 {
            c.is_active = active.0;
        }
        if let Some((pos, size)) = rect.0
            && c.viewport
                .as_ref()
                .is_none_or(|v| v.physical_position != pos || v.physical_size != size)
        {
            c.viewport = Some(Viewport {
                physical_position: pos,
                physical_size: size,
                ..default()
            });
        }
    }
    for mut c in &mut world_cam {
        if c.is_active == active.0 {
            c.is_active = !active.0;
        }
    }
}

/// A model folder's files are watched: a changed file is read again (an
/// export from Blender shows at once, 15D).
fn hot_reload(time: Res<Time>, mut view: ResMut<ModelView>) {
    view.reload_timer += time.delta_secs();
    if view.reload_timer < 1.0 {
        return;
    }
    view.reload_timer = 0.0;
    let Some(o) = &view.open else { return };
    let id = o.id.clone();
    let changed = if o.listed {
        let _ = view.lib.get(&id);
        view.lib.reload_changed().contains(&id)
    } else {
        o.path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .is_some_and(|b| b.len() != o.bytes)
    };
    if changed && view.editing {
        // The document re-imports it (keeping its history).
        view.external_change = true;
    } else if changed {
        let keep = (view.orbit, view.tryon.clone());
        match view.open(&id) {
            Ok(m) => view.status = format!("reloaded {m}"),
            Err(e) => view.status = e,
        }
        view.orbit = keep.0;
        view.tryon = keep.1;
    }
}

/// The triangle mesh of a primitive, its vertex colours × the model's
/// `shade` scale (`extras.ocw.shade_scale`, spec 15G Q11 (a): the plants
/// look as in the game).
fn tri_mesh(p: &ocw_assets::model::Primitive, shade: f32) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.positions.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
    if p.colors.len() == p.positions.len() && !p.colors.is_empty() {
        let colors: Vec<[f32; 4]> = p
            .colors
            .iter()
            .map(|c| ocw_assets::model::shaded(*c, shade))
            .collect();
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    if p.uvs.len() == p.positions.len() && !p.uvs.is_empty() {
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, p.uvs.clone());
    }
    m.insert_indices(Indices::U32(p.indices.clone()));
    m
}

/// The unique edges of a primitive as a line list (the wireframe).
pub fn wire_lines(p: &ocw_assets::model::Primitive) -> Vec<[f32; 3]> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in p.indices.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let k = (a.min(b), a.max(b));
            if seen.insert(k) {
                out.push(p.positions[a as usize]);
                out.push(p.positions[b as usize]);
            }
        }
    }
    out
}

/// The vertex normals as short lines of `len` m.
pub fn normal_lines(p: &ocw_assets::model::Primitive, len: f32) -> Vec<[f32; 3]> {
    p.positions
        .iter()
        .zip(&p.normals)
        .flat_map(|(v, n)| {
            let a = Vec3::from(*v);
            [a.to_array(), (a + Vec3::from(*n) * len).to_array()]
        })
        .collect()
}

fn line_mesh(points: Vec<[f32; 3]>) -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, points);
    m
}

struct Spawn<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    meshes: &'a mut Assets<Mesh>,
    materials: &'a mut Assets<StandardMaterial>,
    mats: &'a ViewMats,
    /// The normals' length (m).
    normal_len: f32,
}

/// Spawns `model` under `parent`: one entity per node (local TRS; for the
/// rider of `body` with its build), meshes, wire and normal lines; returns
/// the node entities. `hidden`: nodes not drawn (and what is below);
/// `tint`: slot colours to use instead of the file's.
fn spawn_model(
    s: &mut Spawn,
    parent: Entity,
    part: Part,
    model: &Model,
    body: Option<&Character>,
    hidden: &[usize],
    tint: &HashMap<String, Option<[f32; 3]>>,
) -> Vec<Entity> {
    let mut out = vec![Entity::PLACEHOLDER; model.nodes.len()];
    let mut material_handles: HashMap<usize, Handle<StandardMaterial>> = HashMap::new();
    for (i, node) in model.nodes.iter().enumerate() {
        let up = node.parent.map_or(parent, |p| out[p]);
        let hide = hidden.iter().any(|&h| model.is_below(i, h));
        let e = s
            .commands
            .spawn((
                Transform {
                    translation: body.map_or(node.translation, |c| {
                        ocw_assets::rider::node_translation(model, i, c)
                    }),
                    rotation: node.rotation,
                    scale: node.scale,
                },
                if hide {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                },
                ChildOf(up),
                Name::new(node.name.clone()),
                ViewNode { part, index: i },
            ))
            .id();
        out[i] = e;
        let level = models::node_level(model, i);
        let (girth, shift) = body.map_or((Vec3::ONE, Vec3::ZERO), |c| {
            ocw_assets::rider::girth_transform(model, i, c)
        });
        for p in &node.primitives {
            let mat = &model.materials[p.material];
            let tinted = tint.get(&mat.name);
            if tinted == Some(&None) {
                continue;
            }
            let base = match tinted {
                Some(Some(c)) => s.materials.add(StandardMaterial {
                    base_color: Color::linear_rgb(c[0], c[1], c[2]),
                    perceptual_roughness: mat.roughness,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                }),
                _ => material_handles
                    .entry(p.material)
                    .or_insert_with(|| {
                        s.materials.add(StandardMaterial {
                            base_color: Color::linear_rgba(
                                mat.color[0],
                                mat.color[1],
                                mat.color[2],
                                mat.color[3],
                            ),
                            metallic: mat.metallic,
                            perceptual_roughness: mat.roughness,
                            emissive: if mat.emissive.iter().any(|c| *c > 0.0) {
                                // The file's own glow (E4 edits it).
                                LinearRgba::rgb(mat.emissive[0], mat.emissive[1], mat.emissive[2])
                                    * 2.0
                            } else if matches!(
                                mat.name.as_str(),
                                "light_front" | "light_rear" | "phone_screen"
                            ) {
                                LinearRgba::rgb(mat.color[0], mat.color[1], mat.color[2]) * 2.0
                            } else {
                                LinearRgba::BLACK
                            },
                            double_sided: true,
                            cull_mode: None,
                            ..default()
                        })
                    })
                    .clone(),
            };
            let at = Transform::from_translation(shift).with_scale(girth);
            let prim = |lines| ViewPrim {
                part,
                node: i,
                material: mat.name.clone(),
                level,
                base: base.clone(),
                lines,
            };
            s.commands.spawn((
                Mesh3d(s.meshes.add(tri_mesh(p, model.shade_scale()))),
                MeshMaterial3d(base.clone()),
                at,
                prim(None),
                ChildOf(e),
                layer(),
            ));
            if part == Part::Main {
                s.commands.spawn((
                    Mesh3d(s.meshes.add(line_mesh(wire_lines(p)))),
                    MeshMaterial3d(s.mats.wire.clone()),
                    at,
                    Visibility::Hidden,
                    prim(Some(true)),
                    ChildOf(e),
                    layer(),
                ));
                s.commands.spawn((
                    Mesh3d(s.meshes.add(line_mesh(normal_lines(p, s.normal_len)))),
                    MeshMaterial3d(s.mats.normal.clone()),
                    at,
                    Visibility::Hidden,
                    prim(Some(false)),
                    ChildOf(e),
                    layer(),
                ));
            }
        }
    }
    out
}

/// The rider's nodes hidden for a look: the optional ones the default
/// character does not show (helmet off for head wear), and the rider's
/// own pieces of the wearable's function (`hair-…` hides `hair_*`).
fn rider_hidden(rider: &Model, wear: Option<&Entry>) -> Vec<usize> {
    let function = wear.and_then(|e| {
        e.file
            .rsplit('/')
            .next()
            .and_then(|n| n.split('-').next())
            .map(str::to_owned)
    });
    let head_wear = wear.is_some_and(|e| e.anchor.as_deref() == Some("head"));
    let variants: Vec<&str> = if head_wear {
        vec![ocw_assets::rider::HELMET_OFF]
    } else {
        Vec::new()
    };
    let shown = ocw_assets::rider::visible(&Character::default(), &variants);
    let mut names: Vec<&str> = ocw_assets::rider::optional()
        .into_iter()
        .filter(|n| !shown.contains(n))
        .collect();
    // Only one hand shape per side.
    names.extend(ocw_assets::rider::hand::others(
        ocw_assets::rider::hand::HandShape::Relaxed,
    ));
    let mut out: Vec<usize> = names.iter().filter_map(|n| rider.node(n)).collect();
    if let Some(f) = function {
        let prefix = format!("{f}_");
        out.extend(
            rider
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.name.starts_with(&prefix))
                .map(|(i, _)| i),
        );
    }
    out
}

/// The plants' slot colours for a season (`ocw_render::flora`; `None`:
/// not drawn, the bare crown of a deciduous tree in winter).
fn season_tint(e: &Entry, season: Season) -> HashMap<String, Option<[f32; 3]>> {
    let mut out = HashMap::new();
    let Some(sp) = e.species.as_deref().and_then(|s| {
        ocw_world::flora::Species::ALL
            .into_iter()
            .find(|x| format!("{x:?}") == s)
    }) else {
        return out;
    };
    let look = season.look();
    let bare = look.leaves < 0.5
        && ocw_render::flora::leaf_type(sp) != ocw_sky::season::LeafType::Evergreen;
    let c = ocw_render::flora::crown_colour(sp, &look, 0.5, 0.5);
    for slot in ["leaf", "needle"] {
        out.insert(slot.to_owned(), if bare { None } else { Some(c) });
    }
    out
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn rebuild(
    mut commands: Commands,
    mut view: ResMut<ModelView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mats: Res<ViewMats>,
    roots: Query<Entity, With<SceneRoot>>,
    mut ground: Query<
        (
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
            &mut Transform,
        ),
        With<ModelGround>,
    >,
) {
    if !view.dirty {
        return;
    }
    view.dirty = false;
    view.style_dirty = true;
    for e in &roots {
        commands.entity(e).despawn();
    }
    let season = match view.tryon {
        TryOn::Scene(s) => Some(s),
        _ => None,
    };
    // The ground under the model: at its origin, or under its lowest
    // point when it reaches below (a rider's feet).
    let floor = match (&view.tryon, view.open.as_ref().and_then(|o| o.stats.bounds)) {
        (TryOn::Off, Some((lo, _))) => lo.y.min(0.0),
        _ => 0.0,
    };
    for (mut vis, mut mat, mut t) in &mut ground {
        t.translation.y = floor;
        *vis = if view.ground {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        mat.0 = match season {
            Some(Season::Winter) => mats.snow.clone(),
            Some(_) => mats.grass.clone(),
            None => mats.ground.clone(),
        };
    }
    let Some(o) = view.open.clone() else { return };
    let root = commands
        .spawn((Transform::default(), Visibility::default(), SceneRoot))
        .id();
    let diag = o
        .stats
        .bounds
        .map_or(1.0, |(lo, hi)| (hi - lo).length())
        .max(0.05);
    let mut s = Spawn {
        commands: &mut commands,
        meshes: &mut meshes,
        materials: &mut materials,
        mats: &mats,
        normal_len: (diag * 0.012).clamp(0.002, 0.2),
    };
    let none = HashMap::new();
    let rider_model = ocw_assets::shipped::rider_model();
    match view.tryon.clone() {
        TryOn::Off => {
            // A rider alone in the default look (the optional pieces of
            // every look are in the file).
            let hidden = if o.entry.kind == Kind::Rider {
                rider_hidden(&o.model, None)
            } else {
                Vec::new()
            };
            spawn_model(&mut s, root, Part::Main, &o.model, None, &hidden, &none);
        }
        TryOn::Rider { shape, height_cm } => {
            let c = models::try_character(shape, height_cm);
            let scale = c.scale();
            let lift = standing_lift(scale);
            let holder = s
                .commands
                .spawn((
                    Transform::from_translation(Vec3::Y * lift),
                    Visibility::default(),
                    ChildOf(root),
                ))
                .id();
            let hidden = rider_hidden(rider_model, Some(&o.entry));
            let nodes = spawn_model(
                &mut s,
                holder,
                Part::Rider,
                rider_model,
                Some(&c),
                &hidden,
                &none,
            );
            if let Some(i) = rider_model.node("rider") {
                s.commands
                    .entity(nodes[i])
                    .insert(Transform::from_scale(Vec3::splat(scale)));
            }
            // Standing at ease, as at the mirror.
            let rig = ocw_assets::shipped::rider_rig();
            let pose = ocw_assets::pose::idle(&rig, 0.0);
            pose_entities(s.commands, rider_model, &nodes, &pose, 1.0, None);
            let anchor = o.entry.anchor.as_deref().unwrap_or("head");
            if let Some(i) = rider_model.node(anchor) {
                let (girth, shift) = ocw_assets::rider::girth_transform(rider_model, i, &c);
                let at = s
                    .commands
                    .spawn((
                        Transform::from_translation(shift).with_scale(girth),
                        Visibility::default(),
                        ChildOf(nodes[i]),
                    ))
                    .id();
                spawn_model(&mut s, at, Part::Main, &o.model, None, &[], &none);
            }
        }
        TryOn::Bike(p) => {
            let bike = ocw_assets::shipped::bike_model(p);
            let nodes = spawn_model(&mut s, root, Part::Bike, bike, None, &[], &none);
            let slot = models::part_slot(&o.entry).unwrap_or("saddle");
            if let Some(i) = models::hotspot_for(bike, slot) {
                spawn_model(&mut s, nodes[i], Part::Main, &o.model, None, &[], &none);
            }
        }
        TryOn::Ride {
            bike: p,
            shape,
            height_cm,
            ..
        } => {
            let c = models::try_character(shape, height_cm);
            let main_is_bike = o.entry.kind == Kind::Bike;
            let bike_model: &Model = if main_is_bike {
                &o.model
            } else {
                ocw_assets::shipped::bike_model(p)
            };
            let rider: &Model = if main_is_bike { rider_model } else { &o.model };
            let (bp, rp) = if main_is_bike {
                (Part::Main, Part::Rider)
            } else {
                (Part::Bike, Part::Main)
            };
            let bike_hidden: Vec<usize> = bike_model.node("lod1").into_iter().collect();
            spawn_model(&mut s, root, bp, bike_model, None, &bike_hidden, &none);
            let hidden = rider_hidden(rider, None);
            let nodes = spawn_model(&mut s, root, rp, rider, Some(&c), &hidden, &none);
            if let Some(i) = rider.node("rider") {
                s.commands
                    .entity(nodes[i])
                    .insert(Transform::from_scale(Vec3::splat(c.scale())));
            }
        }
        TryOn::Scene(season) => {
            let tint = season_tint(&o.entry, season);
            spawn_model(&mut s, root, Part::Main, &o.model, None, &[], &tint);
            // A few more of it around, turned and sized like a ride's.
            let size = o
                .stats
                .bounds
                .map_or(1.0, |(lo, hi)| (hi - lo).xz().length())
                .max(0.3);
            for k in 0..5 {
                let a = k as f32 * 1.3 + 0.4;
                let r = size * (1.4 + 0.35 * k as f32);
                let at = s
                    .commands
                    .spawn((
                        Transform::from_xyz(a.cos() * r, 0.0, a.sin() * r)
                            .with_rotation(Quat::from_rotation_y(a * 2.1))
                            .with_scale(Vec3::splat(0.75 + 0.12 * k as f32)),
                        Visibility::default(),
                        ChildOf(root),
                    ))
                    .id();
                spawn_model(&mut s, at, Part::Copy, &o.model, None, &[], &tint);
            }
        }
    }
}

/// Writes a pose into spawned rider (and bike) nodes.
fn pose_entities(
    commands: &mut Commands,
    rider: &Model,
    nodes: &[Entity],
    pose: &ocw_assets::pose::Pose,
    scale: f32,
    bike: Option<(&Model, &[Entity])>,
) {
    let find = |name: &str| -> Option<Entity> {
        rider
            .node(name)
            .map(|i| nodes[i])
            .or_else(|| bike.and_then(|(m, e)| m.node(name).map(|i| e[i])))
    };
    if let Some(e) = find(ocw_assets::pose::nodes::PELVIS) {
        commands.entity(e).insert(Transform {
            translation: pose.pelvis.0 / scale,
            rotation: pose.pelvis.1,
            scale: Vec3::ONE,
        });
    }
    for (name, q) in pose.rotations() {
        if let (Some(e), Some(i)) = (
            find(name),
            rider
                .node(name)
                .or_else(|| bike.and_then(|(m, _)| m.node(name))),
        ) {
            let m = if rider.node(name).is_some() {
                rider
            } else {
                bike.map(|(m, _)| m).unwrap_or(rider)
            };
            let n = &m.nodes[i];
            commands.entity(e).insert(Transform {
                translation: n.translation,
                rotation: q,
                scale: n.scale,
            });
        }
    }
}

/// The ride try-on: the pose (and the pedalling crank) written into the
/// rider's and the bike's nodes every frame.
fn animate(
    time: Res<Time>,
    mut view: ResMut<ModelView>,
    mut nodes: Query<(&ViewNode, &mut Transform)>,
) {
    let TryOn::Ride {
        bike: p,
        pose,
        shape,
        height_cm,
        pedal,
    } = view.tryon.clone()
    else {
        return;
    };
    let Some(o) = view.open.clone() else { return };
    if pedal {
        // 85 rpm.
        view.crank = (view.crank + time.delta_secs() * 85.0 / 60.0 * std::f32::consts::TAU)
            .rem_euclid(std::f32::consts::TAU);
    }
    let main_is_bike = o.entry.kind == Kind::Bike;
    let bike: &Model = if main_is_bike {
        &o.model
    } else {
        ocw_assets::shipped::bike_model(p)
    };
    let rider: &Model = if main_is_bike {
        ocw_assets::shipped::rider_model()
    } else {
        &o.model
    };
    let c = models::try_character(shape, height_cm);
    let Ok(fit) = models::ride_fit(rider, bike, &c) else {
        return;
    };
    let posed = models::posed_nodes(&fit, &pose.params(view.crank));
    let (bp, rp) = if main_is_bike {
        (Part::Main, Part::Rider)
    } else {
        (Part::Bike, Part::Main)
    };
    let saddle = bike.node(ocw_assets::pose::bike_nodes::SADDLE);
    for (vn, mut t) in &mut nodes {
        let m = if vn.part == rp {
            rider
        } else if vn.part == bp {
            bike
        } else {
            continue;
        };
        let name = m.nodes[vn.index].name.as_str();
        if vn.part == bp && Some(vn.index) == saddle {
            t.translation = m.nodes[vn.index].translation + fit.bike.seat_dir() * fit.saddle_offset;
            continue;
        }
        if let Some((_, at, q)) = posed.iter().find(|(n, _, _)| *n == name) {
            // Bike nodes are posed in the bike, rider nodes in the rider.
            let is_bike_node = bike.node(name).is_some() && rider.node(name).is_none();
            if (vn.part == bp) != is_bike_node {
                continue;
            }
            if let Some(at) = at {
                t.translation = *at;
            }
            t.rotation = *q;
        }
    }
}

/// Visibility by level and toggles, and the highlight.
/// Moves the drawn nodes of the open model to its (edited) transforms.
fn retransform(mut view: ResMut<ModelView>, mut nodes: Query<(&ViewNode, &mut Transform)>) {
    if !std::mem::take(&mut view.retransform) {
        return;
    }
    let Some(o) = &view.open else { return };
    for (n, mut t) in &mut nodes {
        if !matches!(n.part, Part::Main | Part::Copy) {
            continue;
        }
        let Some(node) = o.model.nodes.get(n.index) else {
            continue;
        };
        let want = Transform {
            translation: node.translation,
            rotation: node.rotation,
            scale: node.scale,
        };
        if *t != want {
            *t = want;
        }
    }
}

fn style(
    mut view: ResMut<ModelView>,
    mats: Res<ViewMats>,
    mut prims: Query<(
        &ViewPrim,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
    mut ground: Query<&mut Visibility, (With<ModelGround>, Without<ViewPrim>)>,
) {
    let level = view.level();
    if level != view.shown {
        view.shown = level;
        view.style_dirty = true;
    }
    if !view.style_dirty {
        return;
    }
    view.style_dirty = false;
    for mut v in &mut ground {
        let want = if view.ground {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    let model = view.open.as_ref().map(|o| o.model.clone());
    for (p, mut vis, mut mat) in &mut prims {
        let at_level = match p.level {
            Level::Collision => false,
            l => l == level || (p.part != Part::Main && p.part != Part::Copy && l == Level::Near),
        };
        let shown = at_level
            && match p.lines {
                None => true,
                Some(true) => view.wireframe,
                Some(false) => view.normals,
            };
        let want = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        if p.lines.is_some() {
            continue;
        }
        let lit = p.part == Part::Main
            && match &view.highlight {
                Highlight::None => false,
                Highlight::Material(m) => *m == p.material,
                Highlight::Node(n) => model.as_ref().is_some_and(|m| m.is_below(p.node, *n)),
            };
        let h = if lit {
            mats.highlight.clone()
        } else {
            p.base.clone()
        };
        if mat.0 != h {
            mat.0 = h;
        }
    }
}

/// The orbit camera and the light preset.
#[allow(clippy::type_complexity)]
fn camera_and_light(
    time: Res<Time>,
    mut view: ResMut<ModelView>,
    mut cams: Query<
        (
            &mut Transform,
            &mut Camera,
            &mut AmbientLight,
            &mut Exposure,
        ),
        (With<ModelCamera>, Without<ModelSun>),
    >,
    mut suns: Query<
        (&mut DirectionalLight, &mut Transform),
        (With<ModelSun>, Without<ModelCamera>),
    >,
) {
    if view.turntable {
        let dt = time.delta_secs();
        view.orbit.turn(dt);
    }
    let rig = models::light_rig(view.light);
    let eye = view.orbit.eye();
    let centre = view
        .open
        .as_ref()
        .and_then(|o| o.stats.bounds)
        .map_or(view.orbit.target, |(lo, hi)| (lo + hi) * 0.5);
    view.distance = eye.distance(centre);
    for (mut t, mut cam, mut ambient, mut exposure) in &mut cams {
        let want = Transform::from_translation(eye).looking_at(view.orbit.target, Vec3::Y);
        if *t != want {
            *t = want;
        }
        let sky = Color::srgb(rig.sky_srgb[0], rig.sky_srgb[1], rig.sky_srgb[2]);
        if !matches!(cam.clear_color, ClearColorConfig::Custom(c) if c == sky) {
            cam.clear_color = ClearColorConfig::Custom(sky);
        }
        ambient.brightness = rig.ambient;
        ambient.color =
            Color::linear_rgb(rig.ambient_rgb[0], rig.ambient_rgb[1], rig.ambient_rgb[2]);
        exposure.ev100 = rig.ev100;
    }
    for (mut sun, mut t) in &mut suns {
        sun.illuminance = rig.sun_lux;
        sun.color = Color::linear_rgb(rig.sun_rgb[0], rig.sun_rgb[1], rig.sun_rgb[2]);
        sun.shadow_maps_enabled = rig.shadows;
        let want = Transform::from_translation(rig.sun_dir).looking_at(Vec3::ZERO, Vec3::Y);
        if t.rotation != want.rotation {
            *t = want;
        }
    }
}

/// The gizmo colours.
pub fn gizmo_colour(k: GizmoKind) -> Color {
    match k {
        GizmoKind::Anchor => Color::srgb(1.0, 0.85, 0.1),
        GizmoKind::Hotspot => Color::srgb(0.2, 0.85, 1.0),
        GizmoKind::Collision => Color::srgb(1.0, 0.25, 0.3),
    }
}

/// Where the open model sits in the scene (its root's transform): at the
/// rider's anchor or the bike's hotspot in a try-on.
pub fn main_offset(view: &ModelView) -> Option<Transform> {
    let o = view.open.as_ref()?;
    Some(match &view.tryon {
        TryOn::Rider { shape, height_cm } => {
            let c = models::try_character(*shape, *height_cm);
            let anchor = o.entry.anchor.as_deref().unwrap_or("head");
            let rider = ocw_assets::shipped::rider_model();
            // The rest pose's anchor (the idle pose turns the joints a
            // little: the gizmos are drawn on the rest pose's anchor).
            let a = models::wear_transform(rider, anchor, &c)?;
            Transform::from_matrix(Mat4::from(a))
                .with_translation(Vec3::from(a.translation) + Vec3::Y * standing_lift(c.scale()))
        }
        TryOn::Bike(p) => {
            let bike = ocw_assets::shipped::bike_model(*p);
            let slot = models::part_slot(&o.entry).unwrap_or("saddle");
            Transform::from_matrix(Mat4::from(models::part_transform(bike, slot)?))
        }
        _ => Transform::IDENTITY,
    })
}

/// The edit gizmo of the selected node (E4): its frame in the scene.
pub fn edit_frame(view: &ModelView) -> Option<crate::model_gizmo::Frame> {
    let o = view.open.as_ref()?;
    let i = view.selected.filter(|i| *i < o.model.nodes.len())?;
    let offset = bevy::math::Affine3A::from_mat4(main_offset(view)?.to_matrix());
    let len = o
        .stats
        .bounds
        .map_or(0.3, |(lo, hi)| (hi - lo).length() * 0.16)
        .clamp(0.03, 2.0);
    Some(crate::model_gizmo::frame(
        &o.model,
        i,
        offset,
        view.gizmo_mode,
        len,
    ))
}

/// The edit gizmo's axis colours (x red, y green, z blue; the handle
/// under the pointer or dragged in white).
pub fn axis_colour(axis: usize, hot: bool) -> Color {
    if hot {
        return Color::WHITE;
    }
    match axis {
        0 => Color::srgb(0.95, 0.25, 0.25),
        1 => Color::srgb(0.3, 0.9, 0.3),
        _ => Color::srgb(0.3, 0.5, 1.0),
    }
}

fn draw_edit_gizmo(view: &ModelView, gizmos: &mut Gizmos<ModelGizmos>) {
    use crate::model_gizmo::GizmoMode;
    let Some(f) = edit_frame(view) else { return };
    for axis in 0..3 {
        let c = axis_colour(axis, view.gizmo_hot == Some(axis));
        match view.gizmo_mode {
            GizmoMode::Move => {
                let tip = f.origin + f.axes[axis] * f.len;
                gizmos.arrow(f.origin, tip, c);
            }
            GizmoMode::Scale => {
                let tip = f.origin + f.axes[axis] * f.len;
                gizmos.line(f.origin, tip, c);
                gizmos.cube(
                    Transform::from_translation(tip).with_scale(Vec3::splat(f.len * 0.08)),
                    c,
                );
            }
            GizmoMode::Rotate => {
                gizmos.linestrip(crate::model_gizmo::ring(&f, axis, 64), c);
            }
        }
    }
}

fn draw_gizmos(view: Res<ModelView>, mut gizmos: Gizmos<ModelGizmos>, active: Res<ModelsActive>) {
    if !active.0 {
        return;
    }
    draw_edit_gizmo(&view, &mut gizmos);
    if !view.gizmos {
        return;
    }
    let Some(o) = &view.open else { return };
    let Some(offset) = main_offset(&view) else {
        return;
    };
    let m = offset.to_matrix();
    let r = o
        .stats
        .bounds
        .map_or(0.02, |(lo, hi)| (hi - lo).length() * 0.012)
        .clamp(0.004, 0.2);
    for g in &o.gizmos {
        let c = gizmo_colour(g.kind);
        let t = m * Mat4::from(g.transform);
        let (_, rot, pos) = t.to_scale_rotation_translation();
        match g.kind {
            GizmoKind::Anchor => {
                gizmos.sphere(Isometry3d::new(pos, rot), r, c);
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    gizmos.line(pos, pos + rot * axis * r * 3.0, c);
                }
            }
            GizmoKind::Hotspot => {
                gizmos.sphere(Isometry3d::new(pos, rot), r * 1.3, c);
            }
            GizmoKind::Collision => {
                let box_t = Transform::from_matrix(
                    t * Mat4::from_scale_rotation_translation(
                        g.half * 2.0,
                        Quat::IDENTITY,
                        g.centre,
                    ),
                );
                match g.shape {
                    Some("cylinder") | Some("capsule") => {
                        let (s, q, p) = box_t.to_matrix().to_scale_rotation_translation();
                        for y in [-0.5, 0.5] {
                            let iso = Isometry3d::new(
                                p + q * Vec3::Y * (s.y * y),
                                q * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                            );
                            gizmos.ellipse(iso, Vec2::new(s.x * 0.5, s.z * 0.5), c);
                        }
                        for (dx, dz) in [(0.5, 0.0), (-0.5, 0.0), (0.0, 0.5), (0.0, -0.5)] {
                            let side = q * Vec3::new(s.x * dx, 0.0, s.z * dz);
                            gizmos.line(
                                p + side - q * Vec3::Y * (s.y * 0.5),
                                p + side + q * Vec3::Y * (s.y * 0.5),
                                c,
                            );
                        }
                    }
                    _ => gizmos.cube(box_t, c),
                }
            }
        }
    }
}

/// The labels of the gizmos: (text, window position in logical pixels,
/// colour), for the panels to paint over the view.
pub fn gizmo_labels(
    view: &ModelView,
    camera: &Camera,
    camera_t: &GlobalTransform,
) -> Vec<(String, Vec2, Color)> {
    let mut out = Vec::new();
    if !view.gizmos || !view.labels {
        return out;
    }
    let (Some(o), Some(offset)) = (&view.open, main_offset(view)) else {
        return out;
    };
    let m = offset.to_matrix();
    for g in &o.gizmos {
        let p = m.transform_point3(g.position());
        if let Ok(px) = camera.world_to_viewport(camera_t, p) {
            out.push((g.name.clone(), px, gizmo_colour(g.kind)));
        }
    }
    out
}

/// What a `model …` console line does (pure).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelCmd {
    Open(String),
    List(String),
    Info,
    Show(&'static str, bool),
    Light(LightPreset),
    Lod(LodMode),
    Select(SelectBy),
    Orbit {
        yaw: f32,
        pitch: f32,
        distance: Option<f32>,
    },
    Zoom(f32),
    /// The point the camera orbits and looks at (model space, m): a
    /// close-up of a detail (the "newk it" bike's head tube).
    Target(Vec3),
    Frame,
    Try(TryOn),
    Pose(RidePose),
}

/// A `select` line: a node or material by name (resolved against the
/// open model).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectBy {
    None,
    Node(String),
    Material(String),
}

const TOGGLES: [&str; 6] = [
    "wireframe",
    "normals",
    "ground",
    "gizmos",
    "labels",
    "turntable",
];

fn on_off(w: Option<&String>) -> Result<bool, String> {
    match w.map(String::as_str) {
        None | Some("on") => Ok(true),
        Some("off") => Ok(false),
        Some(x) => Err(format!("`{x}`: on or off")),
    }
}

fn shape_of(w: Option<&String>) -> Result<BodyShape, String> {
    match w {
        None => Ok(BodyShape::Normal),
        Some(s) => BodyShape::parse(s).ok_or_else(|| {
            let all: Vec<&str> = BodyShape::ALL.iter().map(|b| b.id()).collect();
            format!("`{s}`: a body shape ({})", all.join(", "))
        }),
    }
}

fn height_of(w: Option<&String>) -> Result<u16, String> {
    match w {
        None => Ok(ocw_core::character::DEFAULT_HEIGHT_CM),
        Some(s) => s
            .parse::<u16>()
            .map_err(|_| format!("`{s}`: a height in cm"))
            .map(|h| {
                h.clamp(
                    ocw_core::character::HEIGHT_RANGE_CM.0,
                    ocw_core::character::HEIGHT_RANGE_CM.1,
                )
            }),
    }
}

fn profile_of(w: Option<&String>) -> Result<ProfileId, String> {
    let s = w.ok_or("a bike profile is missing (road, road-aero, tt, …)")?;
    ProfileId::parse(s).ok_or_else(|| format!("`{s}`: no bike profile"))
}

fn num(w: Option<&String>, what: &str) -> Result<f32, String> {
    w.ok_or_else(|| format!("{what} is missing"))?
        .parse::<f32>()
        .map_err(|_| format!("{what} must be a number"))
}

/// Parses the words of a `model …` console line.
pub fn parse(words: &[String]) -> Result<ModelCmd, String> {
    let w = |i: usize| words.get(i);
    let rest = |i: usize| words.get(i..).map(|r| r.join(" ")).unwrap_or_default();
    match w(0).map(String::as_str) {
        Some("open") => {
            let what = rest(1);
            if what.is_empty() {
                return Err("open: an id or a file".into());
            }
            Ok(ModelCmd::Open(what))
        }
        Some("list") => Ok(ModelCmd::List(rest(1))),
        Some("info") => Ok(ModelCmd::Info),
        Some("show") => {
            let what =
                w(1).ok_or("show: wireframe, normals, ground, gizmos, labels or turntable")?;
            let t = TOGGLES
                .into_iter()
                .find(|t| t == what)
                .ok_or_else(|| format!("show: `{what}` is none of {}", TOGGLES.join(", ")))?;
            Ok(ModelCmd::Show(t, on_off(w(2))?))
        }
        Some("light") => {
            let p = w(1)
                .and_then(|s| LightPreset::parse(s))
                .ok_or("light: day, dusk, night or overcast")?;
            Ok(ModelCmd::Light(p))
        }
        Some("lod") => {
            let m = w(1)
                .and_then(|s| LodMode::parse(s))
                .ok_or("lod: auto, near, lod1 or billboard")?;
            Ok(ModelCmd::Lod(m))
        }
        Some("select") => match w(1).map(String::as_str) {
            Some("none") | None => Ok(ModelCmd::Select(SelectBy::None)),
            Some("node") => Ok(ModelCmd::Select(SelectBy::Node(rest(2)))),
            Some("material") => Ok(ModelCmd::Select(SelectBy::Material(rest(2)))),
            Some(x) => Err(format!(
                "select: node NAME, material NAME or none, not `{x}`"
            )),
        },
        Some("orbit") => Ok(ModelCmd::Orbit {
            yaw: num(w(1), "yaw (°)")?.to_radians(),
            pitch: num(w(2), "pitch (°)")?.to_radians(),
            distance: w(3).map(|_| num(w(3), "distance (m)")).transpose()?,
        }),
        Some("zoom") => Ok(ModelCmd::Zoom(num(w(1), "distance (m)")?)),
        Some("target") => Ok(ModelCmd::Target(Vec3::new(
            num(w(1), "x (m)")?,
            num(w(2), "y (m)")?,
            num(w(3), "z (m)")?,
        ))),
        Some("frame") => Ok(ModelCmd::Frame),
        Some("pose") => {
            let p = w(1).and_then(|s| RidePose::parse(s)).ok_or_else(|| {
                let all: Vec<&str> = RidePose::ALL.iter().map(|p| p.name()).collect();
                format!("pose: one of {}", all.join(", "))
            })?;
            Ok(ModelCmd::Pose(p))
        }
        Some("try") => match w(1).map(String::as_str) {
            Some("off") | None => Ok(ModelCmd::Try(TryOn::Off)),
            Some("rider") => Ok(ModelCmd::Try(TryOn::Rider {
                shape: shape_of(w(2))?,
                height_cm: height_of(w(3))?,
            })),
            Some("bike") => Ok(ModelCmd::Try(TryOn::Bike(profile_of(w(2))?))),
            Some("ride") => Ok(ModelCmd::Try(TryOn::Ride {
                bike: profile_of(w(2))?,
                pose: match w(3) {
                    Some(p) => RidePose::parse(p).ok_or_else(|| format!("`{p}`: no pose"))?,
                    None => RidePose::Hoods,
                },
                shape: shape_of(w(4))?,
                height_cm: height_of(w(5))?,
                pedal: true,
            })),
            Some("scene") => Ok(ModelCmd::Try(TryOn::Scene(match w(2) {
                Some(s) => Season::parse(s)
                    .ok_or_else(|| format!("`{s}`: spring, summer, autumn or winter"))?,
                None => Season::Summer,
            }))),
            Some(x) => Err(format!("try: off, rider, bike, ride or scene, not `{x}`")),
        },
        Some(x) => Err(format!("model: unknown `{x}`")),
        None => Err(
            "model: open, list, info, show, light, lod, select, orbit, zoom, target, frame, try, pose"
                .into(),
        ),
    }
}

/// The open model's facts and findings, as lines.
pub fn info_lines(view: &ModelView) -> Vec<String> {
    let Some(o) = &view.open else {
        return vec!["no model open".into()];
    };
    let s = &o.stats;
    let (near, far) = o.entry.kind.budget();
    let mut out = vec![
        format!(
            "{} ({}, {})",
            o.id,
            o.entry.kind.name(),
            if o.listed {
                "in the manifest"
            } else {
                "a file"
            }
        ),
        format!(
            "{} nodes, {} materials, {} triangles ({} near of {near}{}), {} vertices, {} bytes",
            s.nodes,
            s.materials,
            s.triangles,
            s.near_triangles,
            match (s.lod1_triangles, far) {
                (Some(l), Some(f)) => format!(", lod1 {l} of {f}"),
                (Some(l), None) => format!(", lod1 {l}"),
                _ => String::new(),
            },
            s.vertices,
            s.file_bytes
        ),
    ];
    if let Some((lo, hi)) = s.bounds {
        out.push(format!(
            "bounds {:.3} … {:.3} m (size {:.3} × {:.3} × {:.3})",
            lo,
            hi,
            hi.x - lo.x,
            hi.y - lo.y,
            hi.z - lo.z
        ));
    }
    out.push(format!(
        "level {:?} at {:.1} m{}",
        view.shown,
        view.distance,
        models::lod_distance(o.entry.kind)
            .map_or(String::new(), |d| format!(" (switch at {d:.0} m)"))
    ));
    for f in &o.findings {
        out.push(f.to_string());
    }
    out
}

/// Carries out a console command on the view.
pub fn run(view: &mut ModelView, cmd: ModelCmd) -> Result<String, String> {
    match cmd {
        ModelCmd::Open(what) => view.open(&what),
        ModelCmd::List(search) => {
            let entries = view.lib.manifest().assets.clone();
            let groups = models::browse(&entries, &search);
            Ok(groups
                .iter()
                .map(|g| {
                    let ids: Vec<&str> = g.entries.iter().map(|e| e.id()).collect();
                    format!("{} ({}): {}", g.folder, ids.len(), ids.join(", "))
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        ModelCmd::Info => Ok(info_lines(view).join("\n")),
        ModelCmd::Show(t, on) => {
            match t {
                "wireframe" => view.wireframe = on,
                "normals" => view.normals = on,
                "ground" => view.ground = on,
                "gizmos" => view.gizmos = on,
                "labels" => view.labels = on,
                _ => view.turntable = on,
            }
            view.style_dirty = true;
            Ok(format!("{t} {}", if on { "on" } else { "off" }))
        }
        ModelCmd::Light(p) => {
            view.light = p;
            Ok(format!("light: {}", p.name()))
        }
        ModelCmd::Lod(m) => {
            view.lod = m;
            view.style_dirty = true;
            Ok(format!("level of detail: {}", m.name()))
        }
        ModelCmd::Select(by) => {
            let o = view.open.as_ref().ok_or("no model open")?;
            view.highlight = match by {
                SelectBy::None => Highlight::None,
                SelectBy::Node(n) => {
                    Highlight::Node(o.model.node(&n).ok_or_else(|| format!("no node `{n}`"))?)
                }
                SelectBy::Material(n) => {
                    o.model
                        .material(&n)
                        .ok_or_else(|| format!("no material `{n}`"))?;
                    Highlight::Material(n)
                }
            };
            view.style_dirty = true;
            Ok(format!("highlight: {:?}", view.highlight))
        }
        ModelCmd::Orbit {
            yaw,
            pitch,
            distance,
        } => {
            view.orbit.yaw = yaw;
            view.orbit.pitch = pitch.clamp(-1.45, 1.45);
            if let Some(d) = distance {
                view.orbit.distance = d.max(0.05);
            }
            Ok(String::new())
        }
        ModelCmd::Zoom(d) => {
            view.orbit.distance = d.max(0.05);
            Ok(format!("distance {d:.1} m"))
        }
        ModelCmd::Target(at) => {
            view.orbit.target = at;
            Ok(String::new())
        }
        ModelCmd::Frame => {
            view.frame();
            Ok(String::new())
        }
        ModelCmd::Try(t) => {
            let o = view.open.as_ref().ok_or("no model open")?;
            let ok = matches!(
                (&t, o.entry.kind),
                (TryOn::Off, _)
                    | (TryOn::Rider { .. }, Kind::Wearable)
                    | (TryOn::Bike(_), Kind::Part)
                    | (TryOn::Ride { .. }, Kind::Rider | Kind::Bike)
                    | (TryOn::Scene(_), Kind::Plant | Kind::Prop)
            );
            if !ok {
                return Err(format!("this try-on is not for a {}", o.entry.kind.name()));
            }
            view.set_tryon(t);
            Ok(format!("try-on: {:?}", view.tryon))
        }
        ModelCmd::Pose(p) => match &mut view.tryon {
            TryOn::Ride { pose, .. } => {
                *pose = p;
                Ok(format!("pose {}", p.name()))
            }
            _ => Err("pose: only in the ride try-on (`model try ride PROFILE`)".into()),
        },
    }
}

fn model_console(world: &mut World, m: &clap::ArgMatches) -> Result<String, String> {
    let words: Vec<String> = m
        .get_many::<String>("words")
        .map(|w| w.cloned().collect())
        .unwrap_or_default();
    let cmd = parse(&words)?;
    let mut view = world.resource_mut::<ModelView>();
    let msg = run(&mut view, cmd)?;
    if !msg.is_empty() {
        view.status = msg.clone();
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn view() -> ModelView {
        ModelView::new(ModelLibrary::embedded(), "embedded".into())
    }

    #[test]
    fn console_lines_parse() {
        assert_eq!(
            parse(&words("open bikes/bike-tt")),
            Ok(ModelCmd::Open("bikes/bike-tt".into()))
        );
        assert_eq!(
            parse(&words("show wireframe")),
            Ok(ModelCmd::Show("wireframe", true))
        );
        assert_eq!(
            parse(&words("show normals off")),
            Ok(ModelCmd::Show("normals", false))
        );
        assert!(parse(&words("show nothing")).is_err());
        assert_eq!(
            parse(&words("light overcast")),
            Ok(ModelCmd::Light(LightPreset::Overcast))
        );
        assert_eq!(parse(&words("lod lod1")), Ok(ModelCmd::Lod(LodMode::Far)));
        assert_eq!(
            parse(&words("target 0 0.8 -0.35")),
            Ok(ModelCmd::Target(Vec3::new(0.0, 0.8, -0.35)))
        );
        assert!(parse(&words("target 0 0.8")).is_err());
        assert_eq!(
            parse(&words("try rider fat 190")),
            Ok(ModelCmd::Try(TryOn::Rider {
                shape: BodyShape::Fat,
                height_cm: 190
            }))
        );
        assert!(matches!(
            parse(&words("try ride tt drops")),
            Ok(ModelCmd::Try(TryOn::Ride {
                bike: ProfileId::TimeTrial,
                pose: RidePose::Drops,
                ..
            }))
        ));
        assert_eq!(
            parse(&words("try scene winter")),
            Ok(ModelCmd::Try(TryOn::Scene(Season::Winter)))
        );
        assert!(parse(&words("try ride")).is_err());
        assert!(
            matches!(parse(&words("orbit 90 10 4")), Ok(ModelCmd::Orbit { distance: Some(d), .. }) if d == 4.0)
        );
        assert_eq!(
            parse(&words("select material frame")),
            Ok(ModelCmd::Select(SelectBy::Material("frame".into())))
        );
    }

    /// The viewer draws a plant's vertex colours × its shade scale, as the
    /// game does (15G Q11 (a)): the file keeps them ≤ 1.
    #[test]
    fn plant_colours_are_drawn_times_the_shade_scale() {
        let lib = ModelLibrary::embedded();
        let m = lib.get("flora/tree-beech-round-a").unwrap();
        let k = m.shade_scale();
        assert!(
            k > 1.0,
            "the beech's crown is shaded above its paint colour"
        );
        let p = &m.nodes[0].primitives[0];
        let mesh = tri_mesh(p, k);
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) =
            mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("no colours");
        };
        for (drawn, stored) in c.iter().zip(&p.colors) {
            assert!(stored[..3].iter().all(|v| *v <= 1.0));
            for i in 0..3 {
                assert!((drawn[i] - stored[i] * k).abs() < 1e-6);
            }
            assert_eq!(drawn[3], stored[3]);
        }
        assert!(c.iter().any(|d| d[1] > 1.0), "brighter than 1 again");
    }

    /// Every committed model opens without an error and with no error
    /// finding (16G).
    #[test]
    fn every_committed_model_opens() {
        let mut v = view();
        let ids: Vec<String> = v
            .lib
            .manifest()
            .assets
            .iter()
            .map(|e| e.id().to_owned())
            .collect();
        assert!(ids.len() > 100);
        for id in &ids {
            v.open(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            let o = v.open.as_ref().unwrap();
            assert!(o.listed);
            let errors: Vec<String> = o
                .findings
                .iter()
                .filter(|f| f.is_error())
                .map(ToString::to_string)
                .collect();
            assert!(errors.is_empty(), "{id}: {errors:#?}");
            assert!(o.stats.triangles > 0, "{id}");
            // Its try-on is one the console accepts.
            if let Some(t) = TryOn::default_for(o.entry.kind, id) {
                run(&mut v, ModelCmd::Try(t)).unwrap_or_else(|e| panic!("{id}: {e}"));
            }
        }
        assert!(v.open("no/such-model").is_err());
    }

    #[test]
    fn the_console_drives_the_view() {
        let mut v = view();
        assert!(run(&mut v, ModelCmd::Select(SelectBy::None)).is_err());
        v.open("bikes/bike-tt").unwrap();
        run(&mut v, ModelCmd::Select(SelectBy::Node("crank".into()))).unwrap();
        let crank = v.open.as_ref().unwrap().model.node("crank").unwrap();
        assert_eq!(v.highlight, Highlight::Node(crank));
        assert!(run(&mut v, ModelCmd::Select(SelectBy::Material("gold".into()))).is_err());
        run(&mut v, ModelCmd::Lod(LodMode::Far)).unwrap();
        assert_eq!(v.level(), Level::Lod1);
        run(&mut v, ModelCmd::Lod(LodMode::Auto)).unwrap();
        v.distance = 60.0;
        assert_eq!(v.level(), Level::Lod1);
        v.distance = 3.0;
        assert_eq!(v.level(), Level::Near);
        // A bike takes the rider; it cannot be worn.
        assert!(
            run(
                &mut v,
                ModelCmd::Try(TryOn::Rider {
                    shape: BodyShape::Normal,
                    height_cm: 178
                })
            )
            .is_err()
        );
        run(
            &mut v,
            ModelCmd::Try(TryOn::default_for(Kind::Bike, "bikes/bike-tt").unwrap()),
        )
        .unwrap();
        assert!(matches!(
            v.tryon,
            TryOn::Ride {
                bike: ProfileId::TimeTrial,
                ..
            }
        ));
        run(&mut v, ModelCmd::Pose(RidePose::Tuck)).unwrap();
        assert!(info_lines(&v).iter().any(|l| l.contains("triangles")));
        let list = run(&mut v, ModelCmd::List("helmet".into())).unwrap();
        assert!(list.starts_with("wear ("), "{list}");
    }

    /// The helmet's gizmo frame is the rider's head for every build: the
    /// scene offset of the open wearable is its anchor (16G).
    #[test]
    fn the_wear_offset_is_the_anchor_for_every_build() {
        let mut v = view();
        v.open("wear/helmet-road-vented").unwrap();
        let rider = ocw_assets::shipped::rider_model();
        for shape in BodyShape::ALL {
            for h in [150, 178, 205] {
                v.set_tryon(TryOn::Rider {
                    shape,
                    height_cm: h,
                });
                let t = main_offset(&v).unwrap();
                let c = models::try_character(shape, h);
                let want = rider.position("head").unwrap() * c.scale()
                    + Vec3::Y * standing_lift(c.scale());
                assert!(t.translation.distance(want) < 1e-4, "{shape:?} {h}");
            }
        }
    }

    #[test]
    fn files_outside_the_folder_get_an_entry_from_their_extras() {
        let m = ocw_assets::shipped::bike_model(ProfileId::Gravel);
        let e = guess_entry(Path::new("/somewhere/else/my-bike.glb"), m);
        assert_eq!(e.kind, Kind::Bike);
        assert_eq!(e.id(), "bikes/bike-gravel");
        assert_eq!(e.profile.as_deref(), Some("gravel"));
        let bare = Model::default();
        let e = guess_entry(Path::new("/x/flora/tree-new-round-a.glb"), &bare);
        assert_eq!(e.kind, Kind::Plant);
        assert_eq!(e.id(), "flora/tree-new-round-a");
        let e = guess_entry(Path::new("/x/y/thing.glb"), &bare);
        assert_eq!(e.kind, Kind::Prop);
    }

    #[test]
    fn wire_and_normal_lines() {
        let m = ocw_assets::shipped::bike_model(ProfileId::Road);
        let p = &m.nodes[m.node("wheel_front").unwrap()].primitives[0];
        let w = wire_lines(p);
        assert_eq!(w.len() % 2, 0);
        // Each edge once: fewer than three per triangle.
        assert!(w.len() / 2 <= p.indices.len());
        let n = normal_lines(p, 0.01);
        assert_eq!(n.len(), p.positions.len() * 2);
        assert!((Vec3::from(n[1]).distance(Vec3::from(n[0])) - 0.01).abs() < 1e-5);
    }

    #[test]
    fn seasons_tint_the_crowns() {
        let lib = ModelLibrary::embedded();
        let e = lib
            .manifest()
            .assets
            .iter()
            .find(|e| e.species.as_deref() == Some("Beech"))
            .cloned()
            .expect("a beech");
        let summer = season_tint(&e, Season::Summer);
        let autumn = season_tint(&e, Season::Autumn);
        assert!(summer["leaf"].is_some());
        assert_ne!(summer["leaf"], autumn["leaf"]);
        assert_eq!(season_tint(&e, Season::Winter)["leaf"], None);
        let pine = lib
            .manifest()
            .assets
            .iter()
            .find(|e| e.species.as_deref() == Some("ScotsPine"))
            .cloned()
            .expect("a pine");
        assert!(season_tint(&pine, Season::Winter)["needle"].is_some());
    }
}
