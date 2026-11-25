//! The studio's panels (spec §14 Phase 16, 16A) with `bevy_egui`: the
//! menu bar (File, Edit with undo / redo, View) and the workspace tabs
//! (Models, World, Routes), the workspace panel on the left, the 2D map
//! on the right, the status line; the 3D view gets the rest of the window
//! (the world camera's viewport). The panels only show the state and
//! **send commands** ([`crate::edit::Pending`]).
//!
//! **Editing a world (E5 step 2, 16C):** a click on a road in the 3D view
//! or on the 2D map selects its edge; the World panel then shows the
//! road's attributes (the package's, and the patch's changes) and edits
//! them, places height points (*Place height point*, then a click on the
//! road), and lists the patch's operations with what the preview found
//! for each. File → Save / Save as / Open patch / Build package.

use bevy::camera::{CameraOutputMode, Viewport, visibility::RenderLayers};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use ocw_edit::world::{RoadChange, WorldDoc, op_road_target};
use ocw_render::camera::FollowCamera;
use ocw_sim::god::FlightModel;
use ocw_world::patch::{Operation, PatchClass, PatchOneway};
use ocw_world::road_surface::RoadSurface;

use crate::build::Sources;
use crate::edit::{
    BuildRequest, PATCH_CLASSES, Pending, PointChange, Studio, StudioCommand, class_name,
    shown_path,
};
use crate::flight::{StudioFlight, UiFocus};
use crate::map::{MapView, Overlay};
use crate::models_ui::ModelsForm;
use crate::models_view::{ModelCamera, ModelView, ModelsActive, ViewportRect};
use crate::patch::{OpStatus, PatchPreview};
use crate::routes::RouteCommand;
use crate::routes_ui::RouteForm;
use crate::world::OpenWorld;

/// The workspaces (16A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Workspace {
    Models,
    #[default]
    World,
    Routes,
}

impl Workspace {
    pub const ALL: [Self; 3] = [Self::Models, Self::World, Self::Routes];

    pub fn name(self) -> &'static str {
        match self {
            Self::Models => "Models",
            Self::World => "World",
            Self::Routes => "Routes",
        }
    }
}

/// What a click in the views does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Tool {
    /// Selects the road or height point there.
    #[default]
    Select,
    /// Places a height point on the selected road.
    PlacePoint,
    /// Moves this height point there.
    MovePoint(String),
    /// Moves this via-point of the route there (Routes).
    RouteMoveVia(usize),
    /// Inserts a via-point there before this one (Routes).
    RouteInsertVia(usize),
}

/// The command a click at `at` (package x, z) sends with `tool` in
/// `workspace`, and the tool afterwards (placing and moving are one click
/// each). In the Routes workspace a plain click selects the via-point
/// there or adds one at the route's end.
pub fn click_command(
    tool: &Tool,
    workspace: Workspace,
    at: [f64; 2],
    tolerance_m: f64,
) -> (StudioCommand, Tool) {
    let route = |c: RouteCommand| (StudioCommand::Route(c), Tool::Select);
    match tool {
        Tool::Select if workspace == Workspace::Routes => {
            route(RouteCommand::Click(at, tolerance_m))
        }
        Tool::Select => (StudioCommand::SelectAt(at, tolerance_m), Tool::Select),
        Tool::PlacePoint => (StudioCommand::AddPoint(Some(at)), Tool::Select),
        Tool::MovePoint(id) => (StudioCommand::MovePoint(id.clone(), at), Tool::Select),
        Tool::RouteMoveVia(i) => route(RouteCommand::MoveVia(*i, at)),
        Tool::RouteInsertVia(i) => route(RouteCommand::InsertVia(*i, at)),
    }
}

/// Whether a press at `p` on the map grabs the route (a via-point or the
/// line within `tol` m) rather than panning.
pub fn grabs_route(view: &crate::routes::RouteView, p: [f64; 2], tol: f64) -> bool {
    if view
        .vias
        .iter()
        .any(|v| (v[0] - p[0]).hypot(v[1] - p[1]) <= tol)
    {
        return true;
    }
    let line: Vec<[f64; 2]> = view.line.iter().map(|q| [q[0], q[2]]).collect();
    line.len() >= 2 && crate::pick::nearest_on_polyline(&line, p).0 <= tol
}

/// The Build dialog's fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildForm {
    pub pbf: String,
    pub poly: String,
    pub hgt: String,
    pub dem: String,
    pub out: String,
}

impl BuildForm {
    /// The fields for the open package: its sources as found next to it.
    pub fn of(studio: &Studio) -> Self {
        let Some(pkg) = studio.package.as_ref() else {
            return Self::default();
        };
        let s = crate::build::find_sources(&pkg.file);
        Self {
            pbf: Sources::list_text(&s.pbf),
            poly: Sources::list_text(&s.poly),
            hgt: Sources::list_text(&s.hgt),
            dem: Sources::list_text(&s.dem),
            out: crate::build::default_output(&pkg.file)
                .display()
                .to_string(),
        }
    }

    pub fn request(&self) -> BuildRequest {
        let list = |t: &str| {
            Sources::parse_list(t)
                .into_iter()
                .map(|p| crate::edit::expand_home(&p.display().to_string()))
                .collect()
        };
        BuildRequest {
            sources: Sources {
                pbf: list(&self.pbf),
                poly: list(&self.poly),
                hgt: list(&self.hgt),
                dem: list(&self.dem),
            },
            out: crate::edit::expand_home(self.out.trim()),
        }
    }
}

/// What the panels show.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct UiState {
    pub workspace: Workspace,
    pub show_map: bool,
    /// The 2D map's layers (16H Q15: the aerial photo off by default).
    pub layers: crate::map::MapLayers,
    /// File → Open's path field (open while `Some`).
    pub open_path: Option<String>,
    /// File → Open patch's path field.
    pub open_patch: Option<String>,
    /// File → Save patch as's path field.
    pub save_as: Option<String>,
    /// The Build dialog (open while `Some`).
    pub build: Option<BuildForm>,
    pub tool: Tool,
    /// The Routes panel's fields.
    pub route_form: RouteForm,
    /// Where the route drag was sent to last (package metres).
    pub drag_sent: Option<[f64; 2]>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            workspace: Workspace::World,
            show_map: true,
            layers: crate::map::MapLayers::default(),
            open_path: None,
            open_patch: None,
            save_as: None,
            build: None,
            tool: Tool::Select,
            route_form: RouteForm::default(),
            drag_sent: None,
        }
    }
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiState>()
            .init_resource::<ModelsForm>()
            .add_systems(Startup, spawn_ui_camera)
            .add_systems(Update, (ui_camera_for_hud, window_title))
            .add_systems(EguiPrimaryContextPass, panels);
    }
}

/// The panels' own camera over the whole window (the world camera draws
/// only into the 3D view's viewport); the console moves egui's primary
/// context to it, the top camera (`ocw_render::console`).
fn spawn_ui_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        RenderLayers::none(),
        Camera {
            order: 10,
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
    ));
}

/// The renderer's HUD (Bevy UI: flight line, FPS, credits) stays in the
/// 3D view.
fn ui_camera_for_hud(mut commands: Commands, cams: Query<Entity, Added<FollowCamera>>) {
    for e in &cams {
        commands.entity(e).insert(IsDefaultUiCamera);
    }
}

/// "OCW Studio - Liechtenstein *" (the star: unsaved changes).
pub fn title(studio: &Studio, world: Option<&OpenWorld>) -> String {
    match world {
        Some(w) => format!(
            "OCW Studio - {}{}",
            w.details.region,
            if studio.session.any_dirty() { " *" } else { "" }
        ),
        None => "OCW Studio".into(),
    }
}

fn window_title(
    studio: Res<Studio>,
    world: Option<Res<OpenWorld>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let t = title(&studio, world.as_deref());
    for mut w in &mut windows {
        if w.title != t {
            w.title = t.clone();
        }
    }
}

/// A path field dialog; the path when it is confirmed.
fn path_dialog(
    ctx: &egui::Context,
    title: &str,
    label: &str,
    button: &str,
    field: &mut Option<String>,
) -> Option<String> {
    let path = field.as_mut()?;
    let mut open = true;
    let mut go = false;
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(label);
            let r = ui.add(egui::TextEdit::singleline(path).desired_width(460.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if ui.button(button).clicked() {
                go = true;
            }
        });
    if go && !path.trim().is_empty() {
        let p = path.clone();
        *field = None;
        Some(p)
    } else {
        if !open {
            *field = None;
        }
        None
    }
}

type ViewCameras<'w, 's> =
    Query<'w, 's, (&'static mut Camera, &'static GlobalTransform), With<FollowCamera>>;

type ModelCameras<'w, 's> = Query<
    'w,
    's,
    (&'static Camera, &'static GlobalTransform),
    (With<ModelCamera>, Without<FollowCamera>),
>;

/// The Models workspace's resources the panels use.
type ModelsUi<'w> = (
    ResMut<'w, ModelView>,
    ResMut<'w, ModelsForm>,
    ResMut<'w, ViewportRect>,
    ResMut<'w, ModelsActive>,
);

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn panels(
    mut contexts: EguiContexts,
    mut state: ResMut<UiState>,
    (studio, preview, menu_view): (
        Res<Studio>,
        Res<PatchPreview>,
        Option<Res<ocw_render::menu::MenuView>>,
    ),
    mut world: Option<ResMut<OpenWorld>>,
    flight: Res<StudioFlight>,
    ground: Option<Res<ocw_render::Ground>>,
    mut pending: ResMut<Pending>,
    mut focus: ResMut<UiFocus>,
    mut cameras: ViewCameras,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
    (mut mview, mut mform, mut vrect, mut mactive): ModelsUi,
    model_cams: ModelCameras,
) -> Result {
    let models_ws = state.workspace == Workspace::Models;
    // The trust dialog (the shared menu, spec 16H Q14) is drawn over the
    // 3D view: the 2D map steps aside meanwhile, so the dialog has the
    // room it has in the game (all of it fits from 4:3 on).
    let dialog = menu_view.is_some_and(|m| m.open);
    if mactive.0 != models_ws {
        mactive.0 = models_ws;
    }
    let ctx = contexts.ctx_mut()?.clone();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "studio".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );
    egui::Panel::top("studio-menu").show(&mut root, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open world...").clicked() {
                    state.open_path =
                        Some(world.as_ref().map(|w| w.spec.clone()).unwrap_or_default());
                    ui.close();
                }
                ui.separator();
                let has_world = studio.world().is_some();
                if ui
                    .add_enabled(has_world, egui::Button::new("Open patch..."))
                    .clicked()
                {
                    state.open_patch = Some(
                        studio
                            .save_path()
                            .map(|p| shown_path(&p))
                            .unwrap_or_default(),
                    );
                    ui.close();
                }
                if ui
                    .add_enabled(
                        has_world,
                        egui::Button::new("Save patch").shortcut_text("Ctrl+S"),
                    )
                    .clicked()
                {
                    pending.send(StudioCommand::SavePatch);
                    ui.close();
                }
                if ui
                    .add_enabled(has_world, egui::Button::new("Save patch as..."))
                    .clicked()
                {
                    state.save_as = Some(
                        studio
                            .save_path()
                            .map(|p| shown_path(&p))
                            .unwrap_or_default(),
                    );
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(has_world, egui::Button::new("Build package..."))
                    .clicked()
                {
                    state.build = Some(BuildForm::of(&studio));
                    ui.close();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    exit.write(AppExit::Success);
                }
            });
            ui.menu_button("Edit", |ui| {
                edit_menu(ui, &studio, &mut pending);
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut state.show_map, "2D map");
                // The map's layers (16H Q15): the aerial photo where the
                // package has one (key O), the neighbours, the border.
                let has_photo = world.as_ref().is_some_and(|w| w.aerial.is_some());
                ui.add_enabled(
                    has_photo,
                    egui::Checkbox::new(&mut state.layers.aerial, "Aerial photo (O)"),
                )
                .on_disabled_hover_text("This package has no aerial photo");
                ui.checkbox(&mut state.layers.neighbours, "Neighbouring packages");
                ui.checkbox(&mut state.layers.border, "World border");
                if ui.button("Fit the map").clicked() {
                    pending.send(StudioCommand::MapFit(false));
                    ui.close();
                }
                if ui.button("Fit the world and its neighbours").clicked() {
                    pending.send(StudioCommand::MapFit(true));
                    ui.close();
                }
                ui.separator();
                ui.label("Flight");
                let model = flight.flight.model;
                if ui
                    .radio(model == FlightModel::Hover, "Hover (stays put)")
                    .clicked()
                {
                    pending.send(StudioCommand::FlightModel(FlightModel::Hover));
                }
                if ui
                    .radio(model == FlightModel::Plane, "Plane (as the unicorn)")
                    .clicked()
                {
                    pending.send(StudioCommand::FlightModel(FlightModel::Plane));
                }
            });
            ui.separator();
            for w in Workspace::ALL {
                if ui
                    .selectable_label(state.workspace == w, w.name())
                    .clicked()
                {
                    state.workspace = w;
                    state.tool = Tool::Select;
                    pending.send(StudioCommand::Workspace(w));
                }
            }
            ui.separator();
            let undo = studio
                .session
                .active()
                .and_then(|d| d.undo_label().map(String::from));
            if ui
                .add_enabled(undo.is_some(), egui::Button::new("Undo"))
                .on_hover_text(undo.as_deref().unwrap_or("nothing to undo"))
                .clicked()
            {
                pending.send(StudioCommand::Undo);
            }
            let redo = studio
                .session
                .active()
                .and_then(|d| d.redo_label().map(String::from));
            if ui
                .add_enabled(redo.is_some(), egui::Button::new("Redo"))
                .on_hover_text(redo.as_deref().unwrap_or("nothing to redo"))
                .clicked()
            {
                pending.send(StudioCommand::Redo);
            }
            match &state.tool {
                Tool::Select => {}
                Tool::PlacePoint => {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 120, 230),
                        "Click the road to place the height point (Esc: cancel)",
                    );
                }
                Tool::MovePoint(id) => {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 120, 230),
                        format!("Click a road to move '{id}' there (Esc: cancel)"),
                    );
                }
                Tool::RouteMoveVia(i) => {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(120, 200, 255),
                        format!("Click where via-point {} goes (Esc: cancel)", i + 1),
                    );
                }
                Tool::RouteInsertVia(i) => {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(120, 200, 255),
                        format!("Click where the new via-point {} goes (Esc: cancel)", i + 1),
                    );
                }
            }
        });
    });
    if state.tool != Tool::Select && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.tool = Tool::Select;
    }
    egui::Panel::bottom("studio-status").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            if models_ws {
                ui.label(crate::models_ui::status(&mview));
            } else {
                ui.label(status_text(&studio));
            }
        });
    });
    egui::Panel::left("studio-workspace")
        .resizable(true)
        .default_size(380.0)
        .show(&mut root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match state.workspace {
                Workspace::World => world_panel(
                    ui,
                    &studio,
                    &preview,
                    world.as_deref(),
                    &mut state.tool,
                    &mut pending,
                ),
                Workspace::Models => crate::models_ui::models_panel(
                    ui,
                    &mut mview,
                    &mut mform,
                    &studio,
                    &mut pending,
                ),
                Workspace::Routes => {
                    let st = &mut *state;
                    crate::routes_ui::routes_panel(
                        ui,
                        &studio,
                        world.as_deref(),
                        &mut st.route_form,
                        &mut st.tool,
                        &mut pending,
                    )
                }
            });
        });
    if state.show_map && !models_ws && !dialog {
        let width = (ctx.viewport_rect().width() * 0.36).max(240.0);
        egui::Panel::right("studio-map")
            .resizable(true)
            .default_size(width)
            .frame(egui::Frame::NONE)
            .show(&mut root, |ui| {
                let rect = ui.available_rect_before_wrap();
                match world.as_mut() {
                    Some(w) => {
                        let routes = state.workspace == Workspace::Routes;
                        let layers = state.layers;
                        let (input, m_per_px) =
                            map_panel(ui, rect, w, &studio, &flight, routes, layers);
                        let tol = crate::pick::map_tolerance_m(m_per_px);
                        // The neighbours are only drawn: a click on one
                        // edits nothing (edits stay in the open package).
                        let foreign = |p: [f64; 2]| {
                            !routes && layers.neighbours && w.surround.in_neighbour(p)
                        };
                        if let Some(d) = input.drag {
                            route_drag(&mut state, &mut pending, d, tol);
                        } else if let Some(p) = input.fly {
                            pending.send(StudioCommand::FlyTo(p, None));
                        } else if let Some(p) = input.click.filter(|p| !foreign(*p)) {
                            let (cmd, next) = click_command(&state.tool, state.workspace, p, tol);
                            pending.send(cmd);
                            state.tool = next;
                        }
                    }
                    None => {
                        ui.centered_and_justified(|ui| ui.label("No world open (File > Open)"));
                    }
                }
            });
    }
    // The 3D view: the rest.
    let central = root.available_rect_before_wrap();
    let view = root.allocate_rect(central, egui::Sense::click_and_drag());
    focus.over_view = (view.hovered() || view.dragged()) && !models_ws;
    if models_ws {
        // The edit gizmo first (E4); the camera orbits otherwise.
        let gizmo = match model_cams.single() {
            Ok(c) => {
                crate::models_ui::edit_input(&ctx, &view, &mut mview, c, &studio, &mut pending)
            }
            Err(_) => false,
        };
        if !gizmo {
            crate::models_ui::view_input(&ctx, &view, &mut mview);
        }
        if let Ok((cam, t)) = model_cams.single() {
            let labels = crate::models_view::gizmo_labels(&mview, cam, t);
            crate::models_ui::paint_labels(&root, central, &labels);
        }
    }
    // A click (not a drag) in the 3D view picks what is under it – not
    // while the trust dialog is open: the click is its.
    if !models_ws
        && !dialog
        && view.clicked()
        && let Some(p) = view.interact_pointer_pos()
        && let Ok((cam, transform)) = cameras.single()
        && let Ok(ray) = cam.viewport_to_world(transform, Vec2::new(p.x, p.y))
    {
        let o = ray.origin;
        let d = ray.direction;
        let ground_fn = |x: f64, z: f64| ground.as_ref().and_then(|g| (g.0)(x, z));
        let hit = crate::pick::ray_ground(
            [f64::from(o.x), f64::from(o.y), f64::from(o.z)],
            [f64::from(d.x), f64::from(d.y), f64::from(d.z)],
            ground_fn,
            30_000.0,
        );
        match hit {
            Some(h) => {
                let dist = ((h[0] - f64::from(o.x)).powi(2)
                    + (h[1] - f64::from(o.y)).powi(2)
                    + (h[2] - f64::from(o.z)).powi(2))
                .sqrt();
                let tol = crate::pick::view_tolerance_m(dist);
                let (cmd, next) = click_command(&state.tool, state.workspace, [h[0], h[2]], tol);
                pending.send(cmd);
                state.tool = next;
            }
            None if state.workspace == Workspace::Routes => {
                pending.send(StudioCommand::Route(RouteCommand::SelectVia(None)));
            }
            None => pending.send(StudioCommand::ClearSelection),
        }
    }
    if let Some(path) = path_dialog(
        &ctx,
        "Open world",
        "World package (a .ocwpkg path, or an installed id):",
        "Open",
        &mut state.open_path,
    ) {
        pending.send(StudioCommand::Open(path));
    }
    if let Some(path) = path_dialog(
        &ctx,
        "Open patch",
        "World patch (.ocwpatch) for the open package:",
        "Open",
        &mut state.open_patch,
    ) {
        pending.send(StudioCommand::OpenPatch(path));
    }
    if let Some(path) = path_dialog(
        &ctx,
        "Save patch as",
        "Save the world patch as (.ocwpatch):",
        "Save",
        &mut state.save_as,
    ) {
        pending.send(StudioCommand::SavePatchAs(path));
    }
    build_window(&ctx, &mut state.build, &studio, &mut pending);
    focus.keyboard = ctx.egui_wants_keyboard_input();
    // The world camera draws into the 3D view only.
    let ppp = ctx.pixels_per_point();
    if let (Ok(window), Ok((mut cam, _))) = (windows.single(), cameras.single_mut()) {
        let max = UVec2::new(window.physical_width(), window.physical_height());
        let pos = UVec2::new(
            (central.min.x * ppp).max(0.0) as u32,
            (central.min.y * ppp).max(0.0) as u32,
        )
        .min(max.saturating_sub(UVec2::ONE));
        let size = UVec2::new(
            (central.width() * ppp) as u32,
            (central.height() * ppp) as u32,
        )
        .min(max - pos)
        .max(UVec2::ONE);
        if vrect.0 != Some((pos, size)) {
            vrect.0 = Some((pos, size));
        }
        let vp = Viewport {
            physical_position: pos,
            physical_size: size,
            ..default()
        };
        if cam
            .viewport
            .as_ref()
            .is_none_or(|v| v.physical_position != pos || v.physical_size != size)
        {
            cam.viewport = Some(vp);
        }
    }
    Ok(())
}

fn edit_menu(ui: &mut egui::Ui, studio: &Studio, pending: &mut Pending) {
    let active = studio.session.active();
    let undo = active.and_then(|d| d.undo_label());
    let redo = active.and_then(|d| d.redo_label());
    let label = |what: &str, l: Option<&str>| match l {
        Some(l) => format!("{what} {l}"),
        None => what.to_string(),
    };
    if ui
        .add_enabled(
            undo.is_some(),
            egui::Button::new(label("Undo", undo)).shortcut_text("Ctrl+Z"),
        )
        .clicked()
    {
        pending.send(StudioCommand::Undo);
        ui.close();
    }
    if ui
        .add_enabled(
            redo.is_some(),
            egui::Button::new(label("Redo", redo)).shortcut_text("Ctrl+Shift+Z"),
        )
        .clicked()
    {
        pending.send(StudioCommand::Redo);
        ui.close();
    }
}

/// The status line: the last message, the active document, the findings
/// and the autosave.
pub fn status_text(studio: &Studio) -> String {
    let mut parts = Vec::new();
    if !studio.status.is_empty() {
        parts.push(studio.status.clone());
    }
    if let Some(d) = studio.session.active() {
        parts.push(format!(
            "{} '{}'{}",
            d.kind(),
            d.name(),
            if d.is_dirty() { ": changed" } else { "" }
        ));
        let n = d.findings().len();
        if n > 0 {
            parts.push(format!("{n} findings"));
        }
    }
    parts.push(match &studio.store_dir {
        Some(dir) => format!("autosave: {}", shown_path(dir)),
        None => "autosave off".into(),
    });
    parts.join("   |   ")
}

/// The colour of an operation's status in the list.
fn status_colour(s: &OpStatus) -> egui::Color32 {
    match s {
        OpStatus::Ok(_) => egui::Color32::from_rgb(110, 210, 110),
        OpStatus::Missing(_) => egui::Color32::from_rgb(255, 90, 80),
        OpStatus::Unchecked(_) | OpStatus::Pending => egui::Color32::from_gray(170),
    }
}

/// The OSM `surface` values offered in the road panel (the patch takes
/// any value the builder knows).
pub const SURFACES: [&str; 12] = [
    "asphalt",
    "concrete",
    "paving_stones",
    "sett",
    "cobblestone",
    "compacted",
    "fine_gravel",
    "gravel",
    "unpaved",
    "dirt",
    "ground",
    "grass",
];

/// The surface's name in the panel.
pub fn surface_name(s: RoadSurface) -> &'static str {
    match s {
        RoadSurface::Asphalt => "asphalt",
        RoadSurface::WornAsphalt => "worn asphalt",
        RoadSurface::Gravel => "gravel",
        RoadSurface::Paving => "paving",
        RoadSurface::Dirt => "dirt",
    }
}

fn oneway_name(o: Option<PatchOneway>) -> &'static str {
    match o {
        None => "package",
        Some(PatchOneway::No) => "no",
        Some(PatchOneway::Forward) => "forward",
        Some(PatchOneway::Backward) => "backward",
    }
}

fn package_oneway(o: ocw_world::graph::Oneway) -> &'static str {
    match o {
        ocw_world::graph::Oneway::No => "no",
        ocw_world::graph::Oneway::Forward => "forward",
        ocw_world::graph::Oneway::Backward => "backward",
    }
}

fn world_panel(
    ui: &mut egui::Ui,
    studio: &Studio,
    preview: &PatchPreview,
    world: Option<&OpenWorld>,
    tool: &mut Tool,
    pending: &mut Pending,
) {
    ui.heading("World");
    let Some(w) = world else {
        ui.label("No world open. File > Open world... (or --world PATH.ocwpkg)");
        return;
    };
    if let Some(r) = &studio.restorable {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(format!("An autosave of this world is there: {}", r.summary));
            ui.horizontal(|ui| {
                if ui.button("Restore").clicked() {
                    pending.send(StudioCommand::RestoreAutosave);
                }
                if ui.button("Discard").clicked() {
                    pending.send(StudioCommand::DiscardAutosave);
                }
            });
        });
    }
    if let Some(doc) = studio.world() {
        if studio.selected_edge().is_some() {
            road_panel(ui, studio, preview, doc, tool, pending);
        } else if let Some(id) = studio.selected_point() {
            point_panel(ui, studio, preview, doc, &id, tool, pending);
        } else {
            ui.label("Click a road in the 3D view or on the map to edit it.");
        }
        ui.add_space(8.0);
        patch_panel(ui, studio, preview, doc, pending);
    }
    ui.add_space(8.0);
    egui::CollapsingHeader::new(egui::RichText::new("Package").strong())
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("package-info")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    for (label, value) in &w.rows {
                        ui.label(label);
                        ui.add(egui::Label::new(value).wrap());
                        ui.end_row();
                    }
                });
        });
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Views").strong());
    ui.label("Saved camera views (studio bookmarks, not part of the package or the patch).");
    if let Some(doc) = studio.world() {
        for v in &doc.content().views {
            ui.horizontal(|ui| {
                let mut name = v.name.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0));
                if r.changed() {
                    pending.send(StudioCommand::RenameView(v.id, name));
                }
                if r.lost_focus() {
                    pending.send(StudioCommand::Seal);
                }
                if ui.small_button("Go").clicked() {
                    pending.send(StudioCommand::GoToView(v.id));
                }
                if ui
                    .small_button("Set")
                    .on_hover_text("Move it to the camera")
                    .clicked()
                {
                    pending.send(StudioCommand::UpdateView(v.id));
                }
                if ui.small_button("Delete").clicked() {
                    pending.send(StudioCommand::DeleteView(v.id));
                }
            });
        }
        if ui.button("Save the camera's view").clicked() {
            pending.send(StudioCommand::AddView(None));
        }
        let findings = doc.findings();
        if !findings.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Findings").strong());
            for f in findings {
                ui.label(format!("{:?}: {}", f.severity, f.message));
            }
        }
    }
}

/// Sends `c`; seals the undo step when a field is left.
fn seal_on(r: &egui::Response, pending: &mut Pending) {
    if r.lost_focus() || r.drag_stopped() {
        pending.send(StudioCommand::Seal);
    }
}

/// The selected road: the package's attributes and the patch's changes.
fn road_panel(
    ui: &mut egui::Ui,
    studio: &Studio,
    preview: &PatchPreview,
    doc: &ocw_edit::Document<WorldDoc>,
    tool: &mut Tool,
    pending: &mut Pending,
) {
    let Some(pkg) = studio.package.as_ref() else {
        return;
    };
    let Some(edge_id) = studio.selected_edge() else {
        return;
    };
    let Some(edge) = pkg.edge(edge_id) else {
        ui.label(format!("edge {edge_id} is not in the package"));
        return;
    };
    let Some(target) = studio.selected_target() else {
        return;
    };
    let content = doc.content();
    let edit = content.road_edit(target);
    let removed = content.road_removal(target);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Road").strong().size(16.0));
            ui.label(format!("edge {edge_id}, OSM way {}", edge.osm_way));
        });
        ui.horizontal(|ui| {
            ui.label("Edits act on");
            if ui.radio(!studio.way_scope, "this edge").clicked() {
                pending.send(StudioCommand::WayScope(false));
            }
            let n = pkg
                .target_edges(ocw_edit::world::RoadTarget::Way(edge.osm_way))
                .len();
            if ui
                .radio(studio.way_scope, format!("the whole way ({n} edges)"))
                .clicked()
            {
                pending.send(StudioCommand::WayScope(true));
            }
        });
        if removed.is_some() {
            ui.colored_label(
                egui::Color32::from_rgb(255, 90, 80),
                format!(
                    "{} is removed from the ridable roads by the patch",
                    target.describe()
                ),
            );
        }
        let r = edit.map(|(_, r)| r);
        let (surface, measured) = pkg.edge_surface(edge);
        egui::Grid::new("road-attributes")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                ui.label("");
                ui.label(egui::RichText::new("package").weak());
                ui.label(egui::RichText::new("patch").weak());
                ui.end_row();
                // Surface.
                ui.label("Surface");
                ui.label(format!(
                    "{}{}",
                    surface_name(surface),
                    if measured { "" } else { " (class)" }
                ));
                let now = r.and_then(|r| r.surface.clone());
                egui::ComboBox::from_id_salt("road-surface")
                    .selected_text(now.as_deref().unwrap_or("-"))
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(now.is_none(), "- (package)").clicked() {
                            pending.send(StudioCommand::SetRoad(RoadChange::Surface(None)));
                        }
                        for s in SURFACES {
                            if ui.selectable_label(now.as_deref() == Some(s), s).clicked() {
                                pending.send(StudioCommand::SetRoad(RoadChange::Surface(Some(
                                    s.into(),
                                ))));
                            }
                        }
                    });
                ui.end_row();
                // Width.
                ui.label("Width");
                ui.label(format!("{:.1} m", edge.width_m));
                ui.horizontal(|ui| {
                    let mut wv = r.and_then(|r| r.width_m).unwrap_or(edge.width_m);
                    let dv = ui.add(
                        egui::DragValue::new(&mut wv)
                            .range(1.0..=30.0)
                            .speed(0.05)
                            .fixed_decimals(1)
                            .suffix(" m"),
                    );
                    if dv.changed() {
                        pending.send(StudioCommand::SetRoad(RoadChange::Width(Some(
                            (wv * 10.0).round() / 10.0,
                        ))));
                    }
                    seal_on(&dv, pending);
                    if r.and_then(|r| r.width_m).is_some() && ui.small_button("x").clicked() {
                        pending.send(StudioCommand::SetRoad(RoadChange::Width(None)));
                    }
                });
                ui.end_row();
                // Class.
                ui.label("Class");
                ui.label(class_name(edge.class));
                let now: Option<PatchClass> = r.and_then(|r| r.class);
                egui::ComboBox::from_id_salt("road-class")
                    .selected_text(now.map_or("-", PatchClass::osm))
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(now.is_none(), "- (package)").clicked() {
                            pending.send(StudioCommand::SetRoad(RoadChange::Class(None)));
                        }
                        for c in PATCH_CLASSES {
                            if ui.selectable_label(now == Some(c), c.osm()).clicked() {
                                pending.send(StudioCommand::SetRoad(RoadChange::Class(Some(c))));
                            }
                        }
                    });
                ui.end_row();
                // One-way.
                ui.label("One-way");
                ui.label(package_oneway(edge.oneway));
                let now = r.and_then(|r| r.oneway);
                egui::ComboBox::from_id_salt("road-oneway")
                    .selected_text(if now.is_none() { "-" } else { oneway_name(now) })
                    .show_ui(ui, |ui| {
                        for o in [
                            None,
                            Some(PatchOneway::No),
                            Some(PatchOneway::Forward),
                            Some(PatchOneway::Backward),
                        ] {
                            let label = if o.is_none() {
                                "- (package)"
                            } else {
                                oneway_name(o)
                            };
                            if ui.selectable_label(now == o, label).clicked() {
                                pending.send(StudioCommand::SetRoad(RoadChange::Oneway(o)));
                            }
                        }
                    });
                ui.end_row();
                // Bike access.
                ui.label("Bikes");
                ui.label("open");
                let mut closed = r.is_some_and(|r| r.bike.is_some());
                if ui.checkbox(&mut closed, "closed for riding").changed() {
                    pending.send(StudioCommand::SetRoad(RoadChange::BikeClosed(closed)));
                }
                ui.end_row();
                // Name.
                ui.label("Name");
                ui.label(edge.name.as_deref().unwrap_or("-"));
                ui.horizontal(|ui| {
                    let mut name = r
                        .and_then(|r| r.name.clone())
                        .or_else(|| edge.name.clone())
                        .unwrap_or_default();
                    let t = ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0));
                    if t.changed() {
                        pending.send(StudioCommand::SetRoad(RoadChange::Name(Some(name))));
                    }
                    seal_on(&t, pending);
                    if r.and_then(|r| r.name.as_ref()).is_some() && ui.small_button("x").clicked() {
                        pending.send(StudioCommand::SetRoad(RoadChange::Name(None)));
                    }
                });
                ui.end_row();
            });
        if let Some((i, _)) = edit {
            let s = preview.preview.status(i);
            ui.colored_label(
                status_colour(&s),
                format!("operation {}: {} {}", i + 1, s.tag(), s.detail()),
            );
        }
        ui.horizontal(|ui| {
            if removed.is_some() {
                if ui.button("Restore road").clicked() {
                    pending.send(StudioCommand::RestoreRoad);
                }
            } else if ui.button("Remove road").clicked() {
                pending.send(StudioCommand::RemoveRoad);
            }
            let placing = *tool == Tool::PlacePoint;
            if ui
                .selectable_label(placing, "Place height point")
                .on_hover_text("Then click the road where the point goes")
                .clicked()
            {
                *tool = if placing {
                    Tool::Select
                } else {
                    Tool::PlacePoint
                };
            }
            if ui.button("Look at it").clicked() {
                pending.send(StudioCommand::Focus);
            }
        });
    });
}

/// The selected height point.
#[allow(clippy::too_many_arguments)] // the panel's parts
fn point_panel(
    ui: &mut egui::Ui,
    studio: &Studio,
    preview: &PatchPreview,
    doc: &ocw_edit::Document<WorldDoc>,
    id: &str,
    tool: &mut Tool,
    pending: &mut Pending,
) {
    let Some((i, h)) = doc.content().height_point(id) else {
        return;
    };
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Height point").strong().size(16.0));
            ui.label(format!("'{id}'"));
        });
        if let Some([lat, lon]) = h.at {
            ui.label(format!(
                "at {lat:.6}, {lon:.6}{}",
                h.edge
                    .map(|e| format!(", on edge {}", e.0))
                    .unwrap_or_default()
            ));
        }
        let absolute = h.height_m.is_some();
        ui.horizontal(|ui| {
            if ui.radio(absolute, "Height").clicked() && !absolute {
                let base = preview_height(studio, h).unwrap_or(0.0);
                pending.send(StudioCommand::ChangePoint(
                    id.into(),
                    PointChange::Height(base + h.raise_m.unwrap_or(0.0)),
                ));
                pending.send(StudioCommand::Seal);
            }
            if ui.radio(!absolute, "Raise").clicked() && absolute {
                pending.send(StudioCommand::ChangePoint(
                    id.into(),
                    PointChange::Raise(0.0),
                ));
                pending.send(StudioCommand::Seal);
            }
            let mut v = h.height_m.or(h.raise_m).unwrap_or(0.0);
            let dv = ui.add(
                egui::DragValue::new(&mut v)
                    .speed(0.05)
                    .fixed_decimals(2)
                    .range(if absolute {
                        -500.0..=9000.0
                    } else {
                        -500.0..=500.0
                    })
                    .suffix(" m"),
            );
            if dv.changed() {
                let c = if absolute {
                    PointChange::Height(v)
                } else {
                    PointChange::Raise(v)
                };
                pending.send(StudioCommand::ChangePoint(id.into(), c));
            }
            seal_on(&dv, pending);
        });
        ui.horizontal(|ui| {
            ui.label("Radius along the road");
            let mut r = h.radius_m.unwrap_or(ocw_world::patch::DEFAULT_RADIUS_M);
            let dv = ui.add(
                egui::DragValue::new(&mut r)
                    .speed(1.0)
                    .range(5.0..=5000.0)
                    .fixed_decimals(0)
                    .suffix(" m"),
            );
            if dv.changed() {
                pending.send(StudioCommand::ChangePoint(
                    id.into(),
                    PointChange::Radius(r),
                ));
            }
            seal_on(&dv, pending);
        });
        let s = preview.preview.status(i);
        ui.colored_label(
            status_colour(&s),
            format!("operation {}: {} {}", i + 1, s.tag(), s.detail()),
        );
        ui.horizontal(|ui| {
            let moving = matches!(tool, Tool::MovePoint(m) if m == id);
            if ui
                .selectable_label(moving, "Move")
                .on_hover_text("Then click the road where it goes")
                .clicked()
            {
                *tool = if moving {
                    Tool::Select
                } else {
                    Tool::MovePoint(id.into())
                };
            }
            if ui.button("Remove").clicked() {
                pending.send(StudioCommand::RemovePoint(id.into()));
            }
            if ui.button("Look at it").clicked() {
                pending.send(StudioCommand::Focus);
            }
        });
    });
}

/// The road's height at a height point as the package has it.
fn preview_height(studio: &Studio, h: &ocw_world::patch::HeightPoint) -> Option<f64> {
    let pkg = studio.package.as_ref()?;
    let p = studio.point_xz(h)?;
    let e = pkg.edge(h.edge?.0)?;
    let (_, arc, _) = crate::pick::nearest_on_polyline(&e.points, p);
    e.height_at(arc)
}

/// The patch: its file and header, and every operation with what the
/// preview found for it (a click selects its target).
fn patch_panel(
    ui: &mut egui::Ui,
    studio: &Studio,
    preview: &PatchPreview,
    doc: &ocw_edit::Document<WorldDoc>,
    pending: &mut Pending,
) {
    let p = &doc.content().patch;
    ui.label(egui::RichText::new("World patch").strong());
    let file = studio
        .patch_file
        .as_ref()
        .map(|f| shown_path(f))
        .unwrap_or_else(|| {
            format!(
                "not saved yet (Save: {})",
                studio
                    .save_path()
                    .map(|f| shown_path(&f))
                    .unwrap_or_default()
            )
        });
    ui.label(format!(
        "{file}{}",
        if doc.is_dirty() { " – changed" } else { "" }
    ));
    let mut info = doc.content().info();
    ui.horizontal(|ui| {
        ui.label("Title");
        let mut t = info.title.clone().unwrap_or_default();
        let r = ui.add(egui::TextEdit::singleline(&mut t).desired_width(220.0));
        if r.changed() {
            info.title = (!t.is_empty()).then_some(t);
            pending.send(StudioCommand::SetInfo(info.clone()));
        }
        seal_on(&r, pending);
    });
    ui.horizontal(|ui| {
        ui.label("Author");
        let mut a = info.author.clone().unwrap_or_default();
        let r = ui.add(egui::TextEdit::singleline(&mut a).desired_width(220.0));
        if r.changed() {
            info.author = (!a.is_empty()).then_some(a);
            pending.send(StudioCommand::SetInfo(info.clone()));
        }
        seal_on(&r, pending);
    });
    if p.ops.is_empty() {
        ui.label("No operations yet.");
    }
    if let Some(n) = &preview.preview.note {
        ui.label(egui::RichText::new(format!("preview: {n}")).weak());
    }
    let sel_edge = studio.selected_edge();
    let sel_point = studio.selected_point();
    egui::Grid::new("patch-ops")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            for (i, op) in p.ops.iter().enumerate() {
                let s = preview.preview.status(i);
                let selected = match op {
                    Operation::HeightPoint(h) => sel_point.as_deref() == Some(h.id.as_str()),
                    _ => op_road_target(op).is_some_and(|t| {
                        sel_edge.is_some_and(|e| {
                            studio
                                .package
                                .as_ref()
                                .and_then(|pk| pk.edge(e))
                                .is_some_and(|edge| t.hits(e, edge.osm_way))
                        })
                    }),
                };
                if ui
                    .selectable_label(selected, format!("{:>2} {}", i + 1, op_summary(op)))
                    .clicked()
                {
                    pending.send(StudioCommand::SelectOp(i));
                }
                ui.colored_label(status_colour(&s), s.tag())
                    .on_hover_text(s.detail());
                if ui
                    .small_button("x")
                    .on_hover_text("Delete this operation")
                    .clicked()
                {
                    pending.send(StudioCommand::DeleteOp(i));
                }
                ui.end_row();
            }
        });
}

/// One line about a patch operation: its kind, target and values.
pub fn op_summary(op: &Operation) -> String {
    let mut parts = vec![op.describe()];
    match op {
        Operation::Road(r) => {
            if let Some(s) = &r.surface {
                parts.push(format!("surface {s}"));
            }
            if let Some(w) = r.width_m {
                parts.push(format!("{w} m"));
            }
            if let Some(c) = r.class {
                parts.push(c.osm().into());
            }
            if let Some(o) = r.oneway {
                parts.push(format!("one-way {}", oneway_name(Some(o))));
            }
            if r.bike.is_some() {
                parts.push("closed for bikes".into());
            }
            if let Some(n) = &r.name {
                parts.push(if n.is_empty() {
                    "no name".into()
                } else {
                    format!("'{n}'")
                });
            }
        }
        Operation::HeightPoint(h) => {
            if let Some(v) = h.height_m {
                parts.push(format!("{v:.1} m"));
            }
            if let Some(v) = h.raise_m {
                parts.push(format!("{v:+.1} m"));
            }
            parts.push(format!(
                "r {:.0} m",
                h.radius_m.unwrap_or(ocw_world::patch::DEFAULT_RADIUS_M)
            ));
        }
        Operation::MoveHeightPoint(m) => {
            if let Some(v) = m.height_m {
                parts.push(format!("{v:.1} m"));
            }
            if let Some(v) = m.raise_m {
                parts.push(format!("{v:+.1} m"));
            }
        }
        Operation::RemoveRoad(_) | Operation::RemoveHeightPoint(_) => {}
    }
    parts.join(", ")
}

/// The Build dialog: the sources (found next to the package or typed),
/// the output, the build's progress and its report.
fn build_window(
    ctx: &egui::Context,
    form: &mut Option<BuildForm>,
    studio: &Studio,
    pending: &mut Pending,
) {
    let Some(f) = form.as_mut() else {
        return;
    };
    let mut open = true;
    egui::Window::new("Build package")
        .open(&mut open)
        .collapsible(false)
        .default_width(620.0)
        .show(ctx, |ui| {
            ui.label("Builds the package again from its source data with the patch (world-builder package --patch), on this computer. The original package stays as it is.");
            egui::Grid::new("build-sources").num_columns(2).show(ui, |ui| {
                for (label, field) in [
                    ("OSM extract (.osm.pbf)", &mut f.pbf),
                    ("Region boundary (.poly)", &mut f.poly),
                    ("SRTM tiles (.hgt)", &mut f.hgt),
                    ("DTM folders (optional)", &mut f.dem),
                    ("Output package", &mut f.out),
                ] {
                    ui.label(label);
                    ui.add(egui::TextEdit::singleline(field).desired_width(420.0));
                    ui.end_row();
                }
            });
            let req = f.request();
            let missing = req.sources.missing();
            if !missing.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 170, 60),
                    "The build needs the source data the package was made from:",
                );
                for m in &missing {
                    ui.label(format!("  - {m}"));
                }
            }
            let program = crate::build::world_builder();
            match &program {
                Some(p) => ui.label(format!("world-builder: {}", shown_path(p))),
                None => ui.colored_label(
                    egui::Color32::from_rgb(255, 170, 60),
                    "world-builder was not found: put it next to ocw-studio, on the PATH, or name it in OCW_WORLD_BUILDER.",
                ),
            };
            let running = studio.build.as_ref().is_some_and(crate::build::BuildJob::running);
            if ui
                .add_enabled(
                    missing.is_empty() && program.is_some() && !running,
                    egui::Button::new("Build"),
                )
                .clicked()
            {
                pending.send(StudioCommand::Build(req));
            }
            if let Some(job) = &studio.build {
                ui.separator();
                ui.label(egui::RichText::new(job.summary()).strong());
                let st = job.state.lock().map(|s| s.lines.clone()).unwrap_or_default();
                let report = crate::build::report_lines(&st);
                if !report.is_empty() {
                    ui.label("Patch report:");
                    for l in &report {
                        let c = if l.starts_with("  MISSING") {
                            egui::Color32::from_rgb(255, 90, 80)
                        } else {
                            egui::Color32::from_gray(220)
                        };
                        ui.colored_label(c, egui::RichText::new(l).monospace());
                    }
                }
                egui::CollapsingHeader::new("Output").show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for l in st.iter().rev().take(400).rev() {
                                ui.label(egui::RichText::new(l).monospace().size(11.0));
                            }
                        });
                });
            }
        });
    if !open {
        *form = None;
    }
}

/// The map panel: what the pointer did, and the zoom (metres per point).
/// A drag of the route on the map: started (grabbing a via-point or the
/// line), moved on (sent again once the pointer moved half a pick
/// tolerance), ended.
fn route_drag(state: &mut UiState, pending: &mut Pending, d: crate::map::MapDrag, tol: f64) {
    use crate::map::DragPhase;
    let route = StudioCommand::Route;
    match d.phase {
        DragPhase::Start => {
            pending.send(route(RouteCommand::DragStart {
                grab: d.from,
                to: d.to,
                tol,
            }));
            state.drag_sent = Some(d.to);
        }
        DragPhase::Move => {
            let far = state
                .drag_sent
                .is_none_or(|p| (p[0] - d.to[0]).hypot(p[1] - d.to[1]) >= tol * 0.5);
            if far {
                pending.send(route(RouteCommand::DragTo(d.to)));
                state.drag_sent = Some(d.to);
            }
        }
        DragPhase::End => {
            if state.drag_sent != Some(d.to) {
                pending.send(route(RouteCommand::DragTo(d.to)));
            }
            pending.send(route(RouteCommand::DragEnd));
            state.drag_sent = None;
        }
    }
}

fn map_panel(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    w: &mut OpenWorld,
    studio: &Studio,
    flight: &StudioFlight,
    routes: bool,
    layers: crate::map::MapLayers,
) -> (crate::map::MapInput, f64) {
    if w.land_texture.is_none()
        && let Some(r) = &w.map.land
    {
        let img = egui::ColorImage::from_rgba_unmultiplied([r.width, r.height], &r.rgba);
        w.land_texture = Some(ui.ctx().load_texture(
            "studio-landuse",
            img,
            egui::TextureOptions::LINEAR,
        ));
    }
    let mut view = w.map_view.unwrap_or_else(|| {
        let (lo, hi) = if std::mem::take(&mut w.fit_all) {
            w.surround.extent(w.map.min, w.map.max)
        } else {
            (w.map.min, w.map.max)
        };
        let v = MapView::fit(lo, hi, [rect.width(), rect.height()]);
        info!(
            "map fitted: centre {:.0}, {:.0}, {:.1} m per point",
            v.centre[0], v.centre[1], v.m_per_px
        );
        v
    });
    let f = &flight.flight;
    // The aerial photo: decoded only while the layer is on.
    let aerial = match w.aerial.as_mut() {
        Some(a) if layers.aerial => a.frame(ui.ctx(), &view, [rect.width(), rect.height()]),
        Some(a) => {
            a.idle();
            Default::default()
        }
        None => Default::default(),
    };
    let overlay = Overlay {
        surround: Some(&w.surround),
        layers,
        aerial,
        camera: Some(([f.pos[0], f.pos[2]], f.view_yaw())),
        views: studio
            .world()
            .map(|d| {
                d.content()
                    .views
                    .iter()
                    .map(|v| ([v.pos[0], v.pos[2]], v.name.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        graph: studio.package.as_ref().and_then(|p| p.graph()),
        marks: if routes {
            crate::overlay::Marks::default()
        } else {
            crate::overlay::marks(studio)
        },
        route: studio.route().map(|_| &studio.routes.view),
        import: if routes {
            studio.routes.pending.as_ref()
        } else {
            None
        },
    };
    let tol = crate::pick::map_tolerance_m(view.m_per_px);
    let route_view = &studio.routes.view;
    let grab = move |p: [f64; 2]| grabs_route(route_view, p, tol);
    let grab: Option<&dyn Fn([f64; 2]) -> bool> =
        (routes && studio.route().is_some()).then_some(&grab as &dyn Fn([f64; 2]) -> bool);
    let input = crate::map::paint(
        ui,
        rect,
        &mut view,
        &w.map,
        w.land_texture.as_ref(),
        &overlay,
        grab,
    );
    w.map_view = Some(view);
    (input, view.m_per_px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_edit::Document;
    use ocw_edit::world::{PackageRef, View, WorldOp};
    use ocw_world::graph::EdgeId;
    use ocw_world::patch::{HeightPoint, RemoveRoad, RoadEdit};

    #[test]
    fn the_title_and_the_status_line_show_changes() {
        let mut s = Studio::new(None, None);
        assert_eq!(title(&s, None), "OCW Studio");
        let id = s
            .session
            .open(|id| Document::new(id, "Testland", WorldDoc::new(PackageRef::default())));
        s.world_doc = Some(id);
        assert_eq!(status_text(&s), "world 'Testland'   |   autosave off");
        s.session
            .execute::<WorldDoc>(id, WorldOp::add(View::named("A")), 0.0)
            .unwrap();
        s.status = "view 'A' saved".into();
        assert_eq!(
            status_text(&s),
            "view 'A' saved   |   world 'Testland': changed   |   autosave off"
        );
        assert_eq!(Workspace::default(), Workspace::World);
        assert_eq!(
            Workspace::ALL.map(Workspace::name),
            ["Models", "World", "Routes"]
        );
    }

    #[test]
    fn a_click_selects_places_or_moves_with_the_tool() {
        let at = [10.0, 20.0];
        let w = Workspace::World;
        assert_eq!(
            click_command(&Tool::Select, w, at, 7.0),
            (StudioCommand::SelectAt(at, 7.0), Tool::Select)
        );
        assert_eq!(
            click_command(&Tool::PlacePoint, w, at, 7.0),
            (StudioCommand::AddPoint(Some(at)), Tool::Select),
            "one click places one point"
        );
        assert_eq!(
            click_command(&Tool::MovePoint("p".into()), w, at, 7.0),
            (StudioCommand::MovePoint("p".into(), at), Tool::Select)
        );
        // In the Routes workspace a click adds or selects a via-point.
        let r = Workspace::Routes;
        assert_eq!(
            click_command(&Tool::Select, r, at, 7.0),
            (
                StudioCommand::Route(RouteCommand::Click(at, 7.0)),
                Tool::Select
            )
        );
        assert_eq!(
            click_command(&Tool::RouteMoveVia(2), r, at, 7.0),
            (
                StudioCommand::Route(RouteCommand::MoveVia(2, at)),
                Tool::Select
            )
        );
        assert_eq!(
            click_command(&Tool::RouteInsertVia(1), r, at, 7.0),
            (
                StudioCommand::Route(RouteCommand::InsertVia(1, at)),
                Tool::Select
            )
        );
    }

    #[test]
    fn a_press_on_the_route_or_a_via_point_grabs_it() {
        let v = crate::routes::RouteView {
            line: vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]],
            vias: vec![[0.0, 0.0], [100.0, 0.0]],
            ..Default::default()
        };
        assert!(grabs_route(&v, [50.0, 3.0], 5.0));
        assert!(grabs_route(&v, [100.0, 4.0], 5.0));
        assert!(!grabs_route(&v, [50.0, 30.0], 5.0));
        assert!(!grabs_route(
            &crate::routes::RouteView::default(),
            [0.0, 0.0],
            5.0
        ));
    }

    #[test]
    fn operations_are_summed_up_in_one_line() {
        let road = Operation::Road(RoadEdit {
            edge: Some(EdgeId(5)),
            surface: Some("gravel".into()),
            width_m: Some(3.5),
            class: Some(PatchClass::Track),
            oneway: Some(PatchOneway::Forward),
            name: Some(String::new()),
            ..Default::default()
        });
        assert_eq!(
            op_summary(&road),
            "road edge 5, surface gravel, 3.5 m, track, one-way forward, no name"
        );
        let point = Operation::HeightPoint(HeightPoint {
            id: "p".into(),
            raise_m: Some(3.0),
            ..Default::default()
        });
        assert_eq!(op_summary(&point), "height_point `p`, +3.0 m, r 100 m");
        let rm = Operation::RemoveRoad(RemoveRoad {
            way: Some(9),
            ..Default::default()
        });
        assert_eq!(op_summary(&rm), "remove_road way 9");
        assert_eq!(surface_name(RoadSurface::Paving), "paving");
        assert!(
            SURFACES
                .iter()
                .all(|s| RoadSurface::from_surface_tag(s).is_some())
        );
    }

    #[test]
    fn the_build_form_reads_its_fields() {
        let f = BuildForm {
            pbf: "a.osm.pbf".into(),
            poly: "a.poly".into(),
            hgt: "x.hgt; y.hgt".into(),
            dem: String::new(),
            out: " out/a.ocwpkg ".into(),
        };
        let r = f.request();
        assert_eq!(r.sources.hgt.len(), 2);
        assert!(r.sources.dem.is_empty());
        assert_eq!(r.out, std::path::PathBuf::from("out/a.ocwpkg"));
        assert_eq!(
            BuildForm::of(&Studio::new(None, None)),
            BuildForm::default()
        );
    }
}
