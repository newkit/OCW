//! The studio's panels (spec §14 Phase 16, 16A) with `bevy_egui`: the
//! menu bar (File, Edit with undo / redo, View) and the workspace tabs
//! (Models, World, Routes), the workspace panel on the left, the 2D map
//! on the right, the status line; the 3D view gets the rest of the window
//! (the world camera's viewport). The panels only show the state and
//! **send commands** ([`crate::edit::Pending`]).

use bevy::camera::{CameraOutputMode, Viewport, visibility::RenderLayers};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use ocw_render::camera::FollowCamera;
use ocw_sim::god::FlightModel;

use crate::edit::{Pending, Studio, StudioCommand};
use crate::flight::{StudioFlight, UiFocus};
use crate::map::{MapView, Overlay};
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

/// What the panels show.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct UiState {
    pub workspace: Workspace,
    pub show_map: bool,
    /// File → Open's path field (open while `Some`).
    pub open_path: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            workspace: Workspace::World,
            show_map: true,
            open_path: None,
        }
    }
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiState>()
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

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn panels(
    mut contexts: EguiContexts,
    mut state: ResMut<UiState>,
    studio: Res<Studio>,
    mut world: Option<ResMut<OpenWorld>>,
    flight: Res<StudioFlight>,
    mut pending: ResMut<Pending>,
    mut focus: ResMut<UiFocus>,
    mut cameras: Query<&mut Camera, With<FollowCamera>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
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
                if ui.button("Quit").clicked() {
                    exit.write(AppExit::Success);
                }
            });
            ui.menu_button("Edit", |ui| {
                edit_menu(ui, &studio, &mut pending);
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut state.show_map, "2D map");
                if ui.button("Fit the map").clicked() {
                    if let Some(w) = world.as_mut() {
                        w.map_view = None;
                    }
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
        });
    });
    egui::Panel::bottom("studio-status").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            ui.label(status_text(&studio));
        });
    });
    egui::Panel::left("studio-workspace")
        .resizable(true)
        .default_size(340.0)
        .show(&mut root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match state.workspace {
                Workspace::World => world_panel(ui, &studio, world.as_deref(), &mut pending),
                Workspace::Models => placeholder(
                    ui,
                    "Models",
                    "The Models workspace comes with stage E3 (view, stats, validator, try-on) and E4 (edit); it needs the model files of Phase 15 (M1, M2).",
                ),
                Workspace::Routes => placeholder(
                    ui,
                    "Routes",
                    "The Routes workspace is the rest of stage E1: GPX import with map matching and problem sections, the built-in planner with via-points and route dragging, the elevation profile, GPX / graph-route export and Test ride (16D).",
                ),
            });
        });
    if state.show_map {
        let width = (ctx.viewport_rect().width() * 0.36).max(240.0);
        egui::Panel::right("studio-map")
            .resizable(true)
            .default_size(width)
            .frame(egui::Frame::NONE)
            .show(&mut root, |ui| {
                let rect = ui.available_rect_before_wrap();
                match world.as_mut() {
                    Some(w) => {
                        let picked = map_panel(ui, rect, w, &studio, &flight);
                        if let Some(p) = picked {
                            pending.send(StudioCommand::FlyTo(p, None));
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
    focus.over_view = view.hovered() || view.dragged();
    if let Some(path) = state.open_path.as_mut() {
        let mut open = true;
        let mut go = false;
        egui::Window::new("Open world")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(&ctx, |ui| {
                ui.label("World package (a .ocwpkg path, or an installed id):");
                let r = ui.add(egui::TextEdit::singleline(path).desired_width(420.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        go = true;
                    }
                });
            });
        if go && !path.trim().is_empty() {
            pending.send(StudioCommand::Open(path.clone()));
            state.open_path = None;
        } else if !open {
            state.open_path = None;
        }
    }
    focus.keyboard = ctx.egui_wants_keyboard_input();
    // The world camera draws into the 3D view only.
    let ppp = ctx.pixels_per_point();
    if let (Ok(window), Ok(mut cam)) = (windows.single(), cameras.single_mut()) {
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
        Some(dir) => format!("autosave: {}", crate::edit::shown_path(dir)),
        None => "autosave off".into(),
    });
    parts.join("   |   ")
}

fn placeholder(ui: &mut egui::Ui, name: &str, text: &str) {
    ui.heading(name);
    ui.add_space(6.0);
    ui.label(text);
}

fn world_panel(
    ui: &mut egui::Ui,
    studio: &Studio,
    world: Option<&OpenWorld>,
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
    ui.label(egui::RichText::new("Package").strong());
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
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Views").strong());
    ui.label("Saved camera views (studio bookmarks, not part of the package).");
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

/// The map panel; the package point double-clicked.
fn map_panel(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    w: &mut OpenWorld,
    studio: &Studio,
    flight: &StudioFlight,
) -> Option<[f64; 2]> {
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
    let mut view = w
        .map_view
        .unwrap_or_else(|| MapView::fit(w.map.min, w.map.max, [rect.width(), rect.height()]));
    let f = &flight.flight;
    let overlay = Overlay {
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
    };
    let picked = crate::map::paint(
        ui,
        rect,
        &mut view,
        &w.map,
        w.land_texture.as_ref(),
        &overlay,
    );
    w.map_view = Some(view);
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_edit::Document;
    use ocw_edit::world::{PackageRef, View, WorldDoc, WorldOp};

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
}
