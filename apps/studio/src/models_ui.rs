//! The Models workspace's panel (spec §14 Phase 16, 16B, stage E3): the
//! model browser (grouped by kind and folder, searched; or a file), the
//! view's switches (light, ground, wireframe, normals, gizmos, level of
//! detail with its switching distance), the stats against the kind's
//! budget, the validator's findings live with their fix hints, the
//! try-on, and the node tree and materials for highlighting. View only:
//! the fixes are applied with stage E4 (edit, with undo).

use bevy::prelude::*;
use bevy_egui::egui;
use ocw_assets::check::{Severity, Target};
use ocw_assets::kinds::Kind;
use ocw_core::{BodyShape, ProfileId};

use crate::models::{self, Level, LightPreset, LodMode, RidePose};
use crate::models_view::{Highlight, ModelCmd, ModelView, Season, TryOn, gizmo_colour, run};

fn colour32(c: Color) -> egui::Color32 {
    let s = c.to_srgba();
    egui::Color32::from_rgb(
        (s.red * 255.0) as u8,
        (s.green * 255.0) as u8,
        (s.blue * 255.0) as u8,
    )
}

const ERROR: egui::Color32 = egui::Color32::from_rgb(235, 90, 80);
const WARNING: egui::Color32 = egui::Color32::from_rgb(235, 180, 60);

/// The panel's own fields.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct ModelsForm {
    /// The *Open file* path field.
    pub file: String,
}

fn bar(ui: &mut egui::Ui, share: f32, text: String) {
    let colour = if share > 1.0 {
        ERROR
    } else if share > 0.85 {
        WARNING
    } else {
        egui::Color32::from_rgb(90, 170, 110)
    };
    ui.add(
        egui::ProgressBar::new(share.clamp(0.0, 1.0))
            .text(text)
            .fill(colour)
            .desired_width(240.0),
    );
}

fn say(view: &mut ModelView, r: Result<String, String>) {
    view.status = match r {
        Ok(m) => m,
        Err(e) => e,
    };
}

/// The Models workspace panel.
pub fn models_panel(ui: &mut egui::Ui, view: &mut ModelView, form: &mut ModelsForm) {
    ui.heading("Models");
    ui.label(format!("Library: {}", view.lib_label));
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.add(egui::TextEdit::singleline(&mut view.search).desired_width(220.0));
        if ui.small_button("x").clicked() {
            view.search.clear();
        }
    });
    let entries = view.lib.manifest().assets.clone();
    let groups = models::browse(&entries, &view.search);
    let mut open: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("model-browser")
        .max_height(180.0)
        .show(ui, |ui| {
            for g in &groups {
                let current = view.open.as_ref().map(|o| o.id.as_str());
                egui::CollapsingHeader::new(format!("{} ({})", g.folder, g.entries.len()))
                    .id_salt(("group", &g.folder))
                    .default_open(!view.search.is_empty() || g.entries.len() <= 8)
                    .show(ui, |ui| {
                        for e in &g.entries {
                            let id = e.id();
                            let name = id.rsplit('/').next().unwrap_or(id);
                            if ui.selectable_label(current == Some(id), name).clicked() {
                                open = Some(id.to_owned());
                            }
                        }
                    });
            }
            if groups.is_empty() {
                ui.label("Nothing matches.");
            }
        });
    ui.horizontal(|ui| {
        ui.label("File");
        ui.add(
            egui::TextEdit::singleline(&mut form.file)
                .hint_text(".glb / .gltf path")
                .desired_width(200.0),
        );
        if ui.button("Open").clicked() && !form.file.trim().is_empty() {
            open = Some(form.file.trim().to_owned());
        }
    });
    if let Some(id) = open {
        let r = view.open(&id);
        say(view, r);
    }
    ui.separator();
    let Some(o) = view.open.clone() else {
        ui.label("Pick a model above.");
        return;
    };
    ui.strong(&o.id);
    ui.label(format!(
        "{} – {}",
        o.entry.kind.name(),
        if o.listed {
            "in the manifest"
        } else {
            "a file outside the model folder (checked as its extras say)"
        }
    ));
    findings_section(ui, view);
    view_section(ui, view);
    stats_section(ui, view);
    tryon_section(ui, view);
    tree_section(ui, view);
}

fn view_section(ui: &mut egui::Ui, view: &mut ModelView) {
    egui::CollapsingHeader::new("View")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut changed = false;
                changed |= ui.checkbox(&mut view.ground, "Ground").changed();
                changed |= ui.checkbox(&mut view.wireframe, "Wireframe").changed();
                changed |= ui.checkbox(&mut view.normals, "Normals").changed();
                changed |= ui.checkbox(&mut view.gizmos, "Gizmos").changed();
                changed |= ui.checkbox(&mut view.labels, "Labels").changed();
                ui.checkbox(&mut view.turntable, "Turntable");
                if changed {
                    view.style_dirty = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Light");
                for p in LightPreset::ALL {
                    if ui.radio(view.light == p, p.name()).clicked() {
                        view.light = p;
                    }
                }
            });
            let Some(o) = &view.open else { return };
            let has_lod1 = o.model.node("lod1").is_some();
            let has_bb = o.model.node("billboard").is_some();
            let kind = o.entry.kind;
            ui.horizontal(|ui| {
                ui.label("Level");
                for m in LodMode::ALL {
                    let enabled = match m {
                        LodMode::Far => has_lod1,
                        LodMode::Billboard => has_bb,
                        _ => true,
                    };
                    let r =
                        ui.add_enabled(enabled, egui::RadioButton::new(view.lod == m, m.name()));
                    if r.clicked() {
                        view.lod = m;
                        view.style_dirty = true;
                    }
                }
            });
            let shown = match view.shown {
                Level::Near => "near",
                Level::Lod1 => "lod1",
                Level::Billboard => "billboard",
                Level::Collision => "collision",
            };
            ui.label(format!(
                "Showing {shown} at {:.1} m; {}",
                view.distance,
                match (models::lod_distance(kind), has_lod1) {
                    (Some(d), true) => format!("the game switches to lod1 at {d:.0} m"),
                    (Some(d), false) =>
                        format!("no lod1 in the file (the game's switch: {d:.0} m)"),
                    (None, true) => "the game has no switch for this kind yet".into(),
                    (None, false) => "no lod1 in the file".into(),
                }
            ));
            if !has_bb && kind == Kind::Plant {
                ui.weak("No billboard in the file yet (15B: later).");
            }
            ui.horizontal(|ui| {
                if ui.button("Frame").clicked() {
                    view.frame();
                }
                ui.weak("Drag: orbit · right / shift-drag: pan · wheel: zoom");
            });
        });
}

fn stats_section(ui: &mut egui::Ui, view: &mut ModelView) {
    let Some(o) = &view.open else { return };
    let s = &o.stats;
    let kind = o.entry.kind;
    egui::CollapsingHeader::new("Stats")
        .default_open(true)
        .show(ui, |ui| {
            let (near, far) = kind.budget();
            if let Some(share) = s.budget_share(kind) {
                let n = if matches!(kind, Kind::Bike | Kind::Rider) {
                    s.triangles
                } else {
                    s.near_triangles
                };
                bar(
                    ui,
                    share,
                    format!("{n} / {near} triangles ({})", kind.name()),
                );
            }
            match (s.lod1_triangles, far) {
                (Some(l), Some(f)) => bar(
                    ui,
                    s.lod1_share().unwrap_or(0.0),
                    format!("lod1 {l} / {f} triangles"),
                ),
                (Some(l), None) => {
                    ui.label(format!("lod1: {l} triangles (no far budget for the kind)"));
                }
                (None, _) => {}
            }
            egui::Grid::new("model-stats")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Triangles");
                    ui.label(format!("{} (near {})", s.triangles, s.near_triangles));
                    ui.end_row();
                    ui.label("Vertices");
                    ui.label(s.vertices.to_string());
                    ui.end_row();
                    ui.label("Nodes / materials");
                    ui.label(format!("{} / {}", s.nodes, s.materials));
                    ui.end_row();
                    if let Some((lo, hi)) = s.bounds {
                        let d = hi - lo;
                        ui.label("Size");
                        ui.label(format!("{:.3} × {:.3} × {:.3} m", d.x, d.y, d.z));
                        ui.end_row();
                        ui.label("Bounds");
                        ui.label(format!(
                            "x {:.3}…{:.3}  y {:.3}…{:.3}  z {:.3}…{:.3}",
                            lo.x, hi.x, lo.y, hi.y, lo.z, hi.z
                        ));
                        ui.end_row();
                    }
                    ui.label("Origin");
                    ui.label(kind.origin());
                    ui.end_row();
                    ui.label("File");
                    let kib = s.file_bytes as f64 / 1024.0;
                    if s.file_bytes > ocw_assets::kinds::WARN_FILE_BYTES {
                        ui.colored_label(WARNING, format!("{kib:.0} KiB (over 2 MB)"));
                    } else {
                        ui.label(format!("{kib:.0} KiB"));
                    }
                    ui.end_row();
                    if o.listed {
                        ui.label("Licence");
                        ui.label(format!("{} – {}", o.entry.license, o.entry.author));
                        ui.end_row();
                    }
                });
        });
}

fn findings_section(ui: &mut egui::Ui, view: &mut ModelView) {
    let Some(o) = view.open.clone() else { return };
    let errors = o.findings.iter().filter(|f| f.is_error()).count();
    let warnings = o.findings.len() - errors;
    egui::CollapsingHeader::new(format!("Validator ({errors} errors, {warnings} warnings)"))
        .id_salt("model-findings")
        .default_open(true)
        .show(ui, |ui| {
            if o.findings.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(110, 200, 120),
                    "The model keeps the contract (15B).",
                );
            }
            for f in &o.findings {
                let c = if f.severity == Severity::Error { ERROR } else { WARNING };
                let r = ui
                    .horizontal_wrapped(|ui| {
                        ui.colored_label(c, if f.is_error() { "error" } else { "warning" });
                        ui.weak(f.code);
                        ui.label(&f.message)
                    })
                    .inner;
                if let Some(fix) = &f.fix {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.weak(format!("fix: {}", fix.describe()));
                    });
                }
                // A click on a finding about a node or a material shows it.
                if r.interact(egui::Sense::click()).clicked() {
                    match &f.target {
                        Target::Node(n) => {
                            if let Some(i) = o.model.node(n) {
                                view.highlight = Highlight::Node(i);
                                view.style_dirty = true;
                            }
                        }
                        Target::Material(m) => {
                            view.highlight = Highlight::Material(m.clone());
                            view.style_dirty = true;
                        }
                        _ => {}
                    }
                }
            }
            ui.weak("The fixes are applied in the edit stage (E4, with undo); `ocw-assets model import` applies the normalising ones.");
        });
}

fn shape_combo(ui: &mut egui::Ui, id: &str, shape: &mut BodyShape) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(shape.name())
        .show_ui(ui, |ui| {
            for s in BodyShape::ALL {
                changed |= ui.selectable_value(shape, s, s.name()).changed();
            }
        });
    changed
}

fn bike_combo(ui: &mut egui::Ui, id: &str, bike: &mut ProfileId) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(bike.name())
        .show_ui(ui, |ui| {
            for p in ProfileId::ALL {
                changed |= ui.selectable_value(bike, p, p.name()).changed();
            }
        });
    changed
}

fn tryon_section(ui: &mut egui::Ui, view: &mut ModelView) {
    let Some(o) = view.open.clone() else { return };
    let kind = o.entry.kind;
    egui::CollapsingHeader::new("Try it on")
        .default_open(true)
        .show(ui, |ui| {
            let Some(default) = TryOn::default_for(kind, &o.id) else {
                ui.weak(match kind {
                    Kind::Creature => "The unicorn is shown in its rest pose; the game turns its joint nodes in flight (the wing and gallop cycles here are open, 16 E3).",
                    _ => "No try-on for this kind yet.",
                });
                return;
            };
            ui.horizontal(|ui| {
                let on = view.tryon != TryOn::Off;
                let label = match kind {
                    Kind::Wearable => "On the rider",
                    Kind::Part => "On a bike",
                    Kind::Rider => "On a bike",
                    Kind::Bike => "With the rider",
                    _ => "In a scene",
                };
                if ui.selectable_label(on, label).clicked() {
                    let r = run(view, ModelCmd::Try(if on { TryOn::Off } else { default.clone() }));
                    say(view, r);
                }
                if ui.selectable_label(!on, "Alone").clicked() && on {
                    let r = run(view, ModelCmd::Try(TryOn::Off));
                    say(view, r);
                }
            });
            let mut t = view.tryon.clone();
            let mut changed = false;
            match &mut t {
                TryOn::Off => {}
                TryOn::Rider { shape, height_cm } => {
                    ui.horizontal(|ui| {
                        ui.label("Body");
                        changed |= shape_combo(ui, "tryon-shape", shape);
                        changed |= ui
                            .add(egui::Slider::new(height_cm, 150..=205).suffix(" cm"))
                            .changed();
                    });
                    ui.weak(format!(
                        "Worn at the rider's `{}` (its girth and the rider's height).",
                        o.entry.anchor.as_deref().unwrap_or("head")
                    ));
                }
                TryOn::Bike(p) => {
                    ui.horizontal(|ui| {
                        ui.label("Bike");
                        changed |= bike_combo(ui, "tryon-bike", p);
                    });
                    match models::part_slot(&o.entry) {
                        Some(slot) => ui.weak(format!("At the bike's hotspot `hs_{slot}` (§8.2).")),
                        None => ui.weak("Not in parts/<slot>/: shown at `hs_saddle`."),
                    };
                }
                TryOn::Ride {
                    bike,
                    pose,
                    shape,
                    height_cm,
                    pedal,
                } => {
                    if kind == Kind::Rider {
                        ui.horizontal(|ui| {
                            ui.label("Bike");
                            changed |= bike_combo(ui, "tryon-ride-bike", bike);
                        });
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Pose");
                        for p in RidePose::ALL {
                            if ui.selectable_label(*pose == p, p.name()).clicked() {
                                *pose = p;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Rider");
                        changed |= shape_combo(ui, "tryon-ride-shape", shape);
                        changed |= ui
                            .add(egui::Slider::new(height_cm, 150..=205).suffix(" cm"))
                            .changed();
                        ui.checkbox(pedal, "Pedal");
                    });
                    ui.weak("The game's fit (the saddle moves with the height) and poses (`ocw_assets::pose`).");
                }
                TryOn::Scene(season) => {
                    ui.horizontal(|ui| {
                        ui.label("Season");
                        for s in Season::ALL {
                            if ui.radio(*season == s, s.name()).clicked() && *season != s {
                                *season = s;
                                changed = true;
                            }
                        }
                    });
                    ui.weak("Weather: the light presets above (overcast = the weather's flat light); no rain or snow particles here.");
                }
            }
            if t != view.tryon {
                view.tryon = t;
                if changed {
                    view.dirty = true;
                }
            }
        });
}

fn tree_section(ui: &mut egui::Ui, view: &mut ModelView) {
    let Some(o) = view.open.clone() else { return };
    egui::CollapsingHeader::new(format!("Materials ({})", o.model.materials.len()))
        .id_salt("model-materials")
        .show(ui, |ui| {
            for m in &o.model.materials {
                let lit = view.highlight == Highlight::Material(m.name.clone());
                let slot = ocw_assets::slots::get(&m.name).is_some();
                let text = if slot {
                    egui::RichText::new(&m.name)
                } else {
                    egui::RichText::new(format!("{} (no paint slot)", m.name)).color(ERROR)
                };
                if ui.selectable_label(lit, text).clicked() {
                    view.highlight = if lit {
                        Highlight::None
                    } else {
                        Highlight::Material(m.name.clone())
                    };
                    view.style_dirty = true;
                }
            }
        });
    let rows = models::tree(&o.model);
    egui::CollapsingHeader::new(format!("Nodes ({})", rows.len()))
        .id_salt("model-nodes")
        .default_open(rows.len() <= 40)
        .show(ui, |ui| {
            for r in rows {
                let lit = view.highlight == Highlight::Node(r.node);
                let mut text = format!(
                    "{}{}",
                    "  ".repeat(r.depth),
                    if r.name.is_empty() {
                        "(unnamed)"
                    } else {
                        &r.name
                    }
                );
                if r.triangles > 0 {
                    text.push_str(&format!("  · {} tri", r.triangles));
                }
                let rich = match r.role {
                    Some("anchor") => egui::RichText::new(format!("{text}  [anchor]"))
                        .color(colour32(gizmo_colour(models::GizmoKind::Anchor))),
                    Some("hotspot") => egui::RichText::new(format!("{text}  [hotspot]"))
                        .color(colour32(gizmo_colour(models::GizmoKind::Hotspot))),
                    Some("collision proxy") => egui::RichText::new(format!("{text}  [collision]"))
                        .color(colour32(gizmo_colour(models::GizmoKind::Collision))),
                    Some(role) => egui::RichText::new(format!("{text}  [{role}]")),
                    None => egui::RichText::new(text),
                };
                let resp = ui.selectable_label(lit, rich.monospace());
                if resp.clicked() {
                    view.highlight = if lit {
                        Highlight::None
                    } else {
                        Highlight::Node(r.node)
                    };
                    view.style_dirty = true;
                }
                if !r.materials.is_empty() {
                    resp.on_hover_text(r.materials.join(", "));
                }
            }
        });
}

/// The mouse over the 3D view: drag orbits, right / shift drag pans,
/// the wheel zooms.
pub fn view_input(ctx: &egui::Context, resp: &egui::Response, view: &mut ModelView) {
    let d = resp.drag_delta();
    if d != egui::Vec2::ZERO {
        let pan = resp.dragged_by(egui::PointerButton::Secondary)
            || resp.dragged_by(egui::PointerButton::Middle)
            || ctx.input(|i| i.modifiers.shift);
        if pan {
            view.orbit.pan(d.x, d.y, resp.rect.height());
        } else {
            view.orbit.drag(d.x, d.y);
        }
    }
    if resp.hovered() {
        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            view.orbit.zoom(scroll / 60.0);
        }
    }
}

/// The gizmos' names painted over the view.
pub fn paint_labels(ui: &egui::Ui, rect: egui::Rect, labels: &[(String, Vec2, Color)]) {
    let painter = ui.painter_at(rect);
    for (text, at, c) in labels {
        let p = egui::pos2(at.x, at.y);
        if !rect.contains(p) {
            continue;
        }
        painter.text(
            p + egui::vec2(6.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            text,
            egui::FontId::proportional(11.0),
            colour32(*c),
        );
    }
}

/// The status line's part of the Models workspace.
pub fn status(view: &ModelView) -> String {
    match &view.open {
        Some(o) => format!(
            "{} · {} triangles · {} · {}",
            o.id,
            o.stats.triangles,
            view.light.name(),
            view.status
        ),
        None => view.status.clone(),
    }
}
