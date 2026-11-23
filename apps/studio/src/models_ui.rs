//! The Models workspace's panel (spec §14 Phase 16, 16B, stage E3): the
//! model browser (grouped by kind and folder, searched; or a file), the
//! view's switches (light, ground, wireframe, normals, gizmos, level of
//! detail with its switching distance), the stats against the kind's
//! budget, the validator's findings live with their fix hints, the
//! try-on, and the node tree and materials for highlighting; and the
//! edits of stage E4 ([`edit_section`]: the selected node's transform,
//! name, parent and delete, the gizmo's mode and snapping, anchors,
//! hotspots, levels and collision proxies added, paint slots and
//! materials, `extras.ocw`, the copyright and the manifest entry, the
//! validator's fixes, save with the diff first, re-import, *Open in
//! Blender*) – each a command of the model document, with undo.

use bevy::prelude::*;
use bevy_egui::egui;
use ocw_assets::check::{Severity, Target};
use ocw_assets::kinds::Kind;
use ocw_core::{BodyShape, ProfileId};

use ocw_edit::model::{ModelDoc, Point, ProxyForm, Trs, contract_names};

use crate::edit::{Pending, Studio, StudioCommand};
use crate::model_edit::ModelCommand;
use crate::model_gizmo::GizmoMode;
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
    /// The edit fields (E4): the new name, the new points' names, the
    /// proxy's shape and form, the extras key and value, the paths.
    pub rename: String,
    pub anchor: String,
    pub hotspot: String,
    pub proxy_shape: String,
    pub proxy_empty: bool,
    pub keep_place: bool,
    pub slot_defaults: bool,
    pub extras_key: String,
    pub extras_value: String,
    pub save_as: String,
    pub reimport: String,
    /// The material whose paint slot picker is open.
    pub slot_picker: Option<usize>,
}

fn send(pending: &mut Pending, c: ModelCommand) {
    pending.send(StudioCommand::Model(c));
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
pub fn models_panel(
    ui: &mut egui::Ui,
    view: &mut ModelView,
    form: &mut ModelsForm,
    studio: &Studio,
    pending: &mut Pending,
) {
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
    let doc = studio.models.document(studio);
    if let Some(d) = doc {
        ui.horizontal(|ui| {
            if d.is_dirty() {
                ui.colored_label(WARNING, "changed");
            } else {
                ui.weak("saved");
            }
            if ui
                .button("Save…")
                .on_hover_text("Shows the model diff against the file first (Ctrl+S)")
                .clicked()
            {
                send(pending, ModelCommand::Save(None));
            }
        });
    }
    findings_section(ui, view, doc.is_some(), pending);
    if let Some(d) = doc {
        edit_section(ui, view, form, studio, d, pending);
    }
    view_section(ui, view);
    stats_section(ui, view);
    tryon_section(ui, view);
    tree_section(ui, view, doc.map(|d| d.content()), pending);
    save_window(ui.ctx(), studio, pending);
}

/// The diff before a save (16B *Save*: `model diff` against the file as
/// opened), with *Save* and *Cancel*.
fn save_window(ctx: &egui::Context, studio: &Studio, pending: &mut Pending) {
    let Some(p) = &studio.models.prompt else {
        return;
    };
    egui::Window::new("Save model")
        .id(egui::Id::new("model-save"))
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(160.0, 0.0))
        .show(ctx, |ui| {
            ui.label(format!("Write {}", crate::edit::shown_path(&p.path)));
            if let Some(m) = &p.manifest_path {
                ui.label(format!("and its entry in {}", crate::edit::shown_path(m)));
            }
            ui.separator();
            ui.strong("model diff against the file as opened");
            egui::ScrollArea::vertical()
                .id_salt("model-save-diff")
                .max_height(260.0)
                .show(ui, |ui| {
                    if p.unchanged {
                        ui.label("no structural change");
                    }
                    for l in &p.lines {
                        let c = if l.starts_with('+') {
                            egui::Color32::from_rgb(110, 200, 120)
                        } else if l.starts_with('-') {
                            ERROR
                        } else {
                            egui::Color32::from_rgb(220, 220, 220)
                        };
                        ui.colored_label(c, egui::RichText::new(l).monospace());
                    }
                });
            if !p.errors.is_empty() {
                ui.separator();
                ui.colored_label(
                    ERROR,
                    format!(
                        "{} contract errors stay in the file (the game refuses it until they are fixed):",
                        p.errors.len()
                    ),
                );
                for e in p.errors.iter().take(6) {
                    ui.colored_label(ERROR, e);
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    send(pending, ModelCommand::ConfirmSave);
                }
                if ui.button("Cancel").clicked() {
                    send(pending, ModelCommand::CancelSave);
                }
            });
        });
}

fn xyz(ui: &mut egui::Ui, label: &str, v: &mut Vec3, speed: f64) -> (bool, bool) {
    let mut changed = false;
    let mut done = false;
    ui.horizontal(|ui| {
        ui.label(label);
        for c in [&mut v.x, &mut v.y, &mut v.z] {
            let r = ui.add(egui::DragValue::new(c).speed(speed).max_decimals(4));
            changed |= r.changed();
            done |= r.drag_stopped() || r.lost_focus();
        }
    });
    (changed, done)
}

/// The edit tools (E4).
fn edit_section(
    ui: &mut egui::Ui,
    view: &mut ModelView,
    form: &mut ModelsForm,
    studio: &Studio,
    doc: &ocw_edit::Document<ModelDoc>,
    pending: &mut Pending,
) {
    let d = doc.content();
    let sel = studio.models.selected(studio);
    egui::CollapsingHeader::new("Edit")
        .id_salt("model-edit")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Gizmo");
                for m in GizmoMode::ALL {
                    if ui.radio(studio.models.gizmo == m, m.name()).clicked() {
                        send(pending, ModelCommand::Gizmo(m));
                    }
                }
                let mut snap = studio.models.snapping;
                if ui
                    .checkbox(&mut snap, "Snap")
                    .on_hover_text("5 mm, 7.5°, scale steps of 0.05")
                    .changed()
                {
                    send(pending, ModelCommand::Snapping(snap));
                }
            });
            let Some(id) = sel else {
                ui.weak("Select a node in the tree or click it in the view.");
                points_section(ui, form, d, None, pending);
                return;
            };
            let Some(node) = d.node(id) else {
                return;
            };
            ui.horizontal(|ui| {
                ui.strong(&node.name);
                if let Some(role) = ocw_assets::check::contract_role(&node.name) {
                    ui.weak(format!("[{role}]"));
                }
                if ui.small_button("Deselect").clicked() {
                    send(pending, ModelCommand::Select(None));
                }
            });
            // The transform: the gizmo's numbers.
            let trs = Trs::of(node);
            let mut t = trs.translation;
            let (e0, e1, e2) = trs.rotation.to_euler(EulerRot::XYZ);
            let mut r = Vec3::new(e0, e1, e2) * (180.0 / std::f32::consts::PI);
            let mut sc = trs.scale;
            let (c1, d1) = xyz(ui, "Move   ", &mut t, 0.001);
            let (c2, d2) = xyz(ui, "Rotate°", &mut r, 0.5);
            let (c3, d3) = xyz(ui, "Scale  ", &mut sc, 0.005);
            if c1 || c2 || c3 {
                let rr = r * (std::f32::consts::PI / 180.0);
                let next = Trs {
                    translation: t,
                    rotation: if c2 {
                        Quat::from_euler(EulerRot::XYZ, rr.x, rr.y, rr.z)
                    } else {
                        trs.rotation
                    },
                    scale: sc.max(Vec3::splat(1e-4)),
                };
                send(pending, ModelCommand::Transform(id, next));
            }
            if d1 || d2 || d3 {
                send(pending, ModelCommand::Seal);
            }
            // The name: a contract name or free.
            ui.horizontal(|ui| {
                ui.label("Name");
                egui::ComboBox::from_id_salt("model-rename-pick")
                    .selected_text("contract names")
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for n in contract_names(d.entry.kind) {
                            let taken = d.by_name(&n).is_some();
                            if ui
                                .add_enabled(!taken, egui::Button::selectable(false, n.as_str()))
                                .clicked()
                            {
                                form.rename.clone_from(&n);
                            }
                        }
                    });
                ui.add(egui::TextEdit::singleline(&mut form.rename).desired_width(110.0));
                if ui.button("Rename").clicked() && !form.rename.trim().is_empty() {
                    send(
                        pending,
                        ModelCommand::Rename(id, form.rename.trim().to_owned()),
                    );
                }
            });
            // The parent.
            ui.horizontal(|ui| {
                ui.label("Parent");
                let current = d.parent_of(id);
                let label = current.map_or_else(
                    || "(root)".into(),
                    |p| d.node(p).map(|n| n.name.clone()).unwrap_or_default(),
                );
                let below = d.subtree(id);
                egui::ComboBox::from_id_salt("model-parent")
                    .selected_text(label)
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_none(), "(root)").clicked() {
                            send(pending, ModelCommand::Reparent(id, None, form.keep_place));
                        }
                        for (i, n) in d.model.nodes.iter().enumerate() {
                            let pid = d.ids[i];
                            if below.contains(&pid) {
                                continue;
                            }
                            if ui.selectable_label(current == Some(pid), &n.name).clicked() {
                                send(
                                    pending,
                                    ModelCommand::Reparent(id, Some(pid), form.keep_place),
                                );
                            }
                        }
                    });
                ui.checkbox(&mut form.keep_place, "keep its place");
            });
            let n = d.subtree(id).len();
            let text = if n > 1 {
                format!("Delete (with {} below)", n - 1)
            } else {
                "Delete".into()
            };
            if ui.button(text).clicked() {
                send(pending, ModelCommand::Delete(id));
            }
            points_section(ui, form, d, Some(id), pending);
        });
    materials_section(ui, view, form, d, pending);
    metadata_section(ui, form, d, pending);
    files_section(ui, form, studio, pending);
}

/// Anchors, hotspots, levels and collision proxies added (under the
/// selected node).
fn points_section(
    ui: &mut egui::Ui,
    form: &mut ModelsForm,
    d: &ModelDoc,
    sel: Option<ocw_edit::model::NodeId>,
    pending: &mut Pending,
) {
    ui.separator();
    let under = sel
        .and_then(|i| d.node(i))
        .map_or_else(|| "the root".into(), |n| n.name.clone());
    ui.label(format!("Add under {under}"));
    ui.horizontal(|ui| {
        ui.label("anchor_");
        ui.add(
            egui::TextEdit::singleline(&mut form.anchor)
                .desired_width(90.0)
                .hint_text("bottle"),
        );
        if ui.button("Add anchor").clicked() && !form.anchor.trim().is_empty() {
            send(
                pending,
                ModelCommand::AddPoint(Point::Anchor(form.anchor.trim().into()), sel, [0.0; 3]),
            );
        }
    });
    if d.entry.kind == Kind::Bike {
        ui.horizontal(|ui| {
            ui.label("Hotspot");
            egui::ComboBox::from_id_salt("model-hotspot")
                .selected_text(if form.hotspot.is_empty() {
                    "slot"
                } else {
                    form.hotspot.as_str()
                })
                .show_ui(ui, |ui| {
                    for h in ocw_assets::pose::bike_nodes::HOTSPOTS {
                        let slot = h.trim_start_matches("hs_");
                        let taken = d.model.node(h).is_some();
                        if ui
                            .add_enabled(
                                !taken,
                                egui::Button::selectable(form.hotspot == slot, slot),
                            )
                            .clicked()
                        {
                            form.hotspot = slot.into();
                        }
                    }
                });
            if ui.button("Add hotspot").clicked() && !form.hotspot.is_empty() {
                send(
                    pending,
                    ModelCommand::AddPoint(Point::Hotspot(form.hotspot.clone()), sel, [0.0; 3]),
                );
            }
        });
    }
    ui.horizontal(|ui| {
        ui.label("Level");
        let levels: &[&str] = if d.entry.kind == Kind::Plant {
            &["lod1", "lod2", "billboard"]
        } else {
            &["lod1"]
        };
        for l in levels {
            let taken = d.model.node(l).is_some();
            if ui
                .add_enabled(!taken, egui::Button::new(format!("Add {l}")))
                .clicked()
            {
                send(
                    pending,
                    ModelCommand::AddPoint(Point::Level((*l).into()), None, [0.0; 3]),
                );
            }
        }
    });
    if form.proxy_shape.is_empty() {
        form.proxy_shape = "box".into();
    }
    ui.horizontal(|ui| {
        ui.label("Proxy");
        for shape in ocw_assets::check::COLLISION_SHAPES {
            ui.radio_value(&mut form.proxy_shape, shape.to_string(), shape);
        }
    });
    ui.horizontal(|ui| {
        ui.radio_value(&mut form.proxy_empty, false, "mesh")
            .on_hover_text("A mesh whose bounds are the shape");
        ui.radio_value(&mut form.proxy_empty, true, "scaled empty")
            .on_hover_text("An empty: the unit shape scaled by the node (15H Q9: both forms stay)");
        if ui
            .button("Add proxy")
            .on_hover_text("Around the selected node's mesh, else the whole model")
            .clicked()
        {
            let m = &d.model;
            let b = sel
                .and_then(|id| d.index_of(id))
                .and_then(|i| m.bounds_where(|j| m.is_below(j, i)))
                .or_else(|| m.near_bounds())
                .unwrap_or((Vec3::splat(-0.5), Vec3::splat(0.5)));
            send(
                pending,
                ModelCommand::AddProxy {
                    shape: form.proxy_shape.clone(),
                    form: if form.proxy_empty {
                        ProxyForm::Empty
                    } else {
                        ProxyForm::Mesh
                    },
                    parent: None,
                    centre: ((b.0 + b.1) * 0.5).to_array(),
                    half: ((b.1 - b.0) * 0.5).max(Vec3::splat(0.01)).to_array(),
                },
            );
        }
    });
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The paint slot picker of material `i`: every slot of
/// `ocw_assets::slots` with its default colour; one another material has
/// already merges the two; tinted slots (the rider's look recolours them)
/// marked.
fn slot_picker(
    ui: &mut egui::Ui,
    d: &ModelDoc,
    i: usize,
    form: &mut ModelsForm,
    pending: &mut Pending,
) {
    let current = d.model.materials[i].name.as_str();
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.weak(format!(
            "Paint slot for `{current}` ({} slots; * tinted by the look, + merges)",
            ocw_assets::slots::ALL.len()
        ));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
            for s in ocw_assets::slots::ALL {
                let used = d.model.material(s.name).is_some_and(|j| j != i);
                let c = ocw_assets::model::srgb(s.color);
                let swatch = egui::Color32::from_rgb(
                    (linear_to_srgb(c[0]) * 255.0) as u8,
                    (linear_to_srgb(c[1]) * 255.0) as u8,
                    (linear_to_srgb(c[2]) * 255.0) as u8,
                );
                let tinted = s.tint != ocw_assets::slots::Tint::Fixed;
                let text = format!(
                    "{}{}{}",
                    s.name,
                    if tinted { "*" } else { "" },
                    if used { "+" } else { "" }
                );
                let r = ui.add(
                    egui::Button::selectable(current == s.name, egui::RichText::new(text).small())
                        .fill(swatch.gamma_multiply(0.45)),
                );
                if r.clicked() {
                    send(
                        pending,
                        ModelCommand::Slot(i, s.name.into(), form.slot_defaults),
                    );
                    form.slot_picker = None;
                }
            }
        });
    });
}

/// Paint slots and the materials' default colours (16B *Materials*).
fn materials_section(
    ui: &mut egui::Ui,
    view: &mut ModelView,
    form: &mut ModelsForm,
    d: &ModelDoc,
    pending: &mut Pending,
) {
    egui::CollapsingHeader::new(format!(
        "Materials and paint slots ({})",
        d.model.materials.len()
    ))
    .id_salt("model-edit-materials")
    .default_open(true)
    .show(ui, |ui| {
        ui.checkbox(
            &mut form.slot_defaults,
            "a picked slot brings its default colours",
        );
        for (i, m) in d.model.materials.iter().enumerate() {
            ui.push_id(("mat", i), |ui| {
                ui.horizontal(|ui| {
                    let lit = view.highlight == Highlight::Material(m.name.clone());
                    let is_slot = ocw_assets::slots::get(&m.name).is_some();
                    let text = if is_slot {
                        egui::RichText::new(&m.name)
                    } else {
                        egui::RichText::new(format!("{} (no slot)", m.name)).color(ERROR)
                    };
                    if ui.selectable_label(lit, text).clicked() {
                        view.highlight = if lit {
                            Highlight::None
                        } else {
                            Highlight::Material(m.name.clone())
                        };
                        view.style_dirty = true;
                    }
                    let open = form.slot_picker == Some(i);
                    if ui
                        .selectable_label(open, "paint slot…")
                        .on_hover_text("Pick the paint slot (ocw_assets::slots) of this material")
                        .clicked()
                    {
                        form.slot_picker = if open { None } else { Some(i) };
                    }
                });
                if form.slot_picker == Some(i) {
                    slot_picker(ui, d, i, form, pending);
                }
                ui.horizontal(|ui| {
                    let mut c = [
                        linear_to_srgb(m.color[0]),
                        linear_to_srgb(m.color[1]),
                        linear_to_srgb(m.color[2]),
                    ];
                    let mut e = m.emissive.map(linear_to_srgb);
                    let mut met = m.metallic;
                    let mut rough = m.roughness;
                    ui.label("colour");
                    let r1 = ui.color_edit_button_rgb(&mut c);
                    ui.label("glow");
                    let r2 = ui.color_edit_button_rgb(&mut e);
                    let r3 = ui.add(
                        egui::DragValue::new(&mut met)
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .prefix("metal "),
                    );
                    let r4 = ui.add(
                        egui::DragValue::new(&mut rough)
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .prefix("rough "),
                    );
                    if r1.changed() || r2.changed() || r3.changed() || r4.changed() {
                        let mut next = m.clone();
                        if r1.changed() {
                            next.color = [
                                srgb_to_linear(c[0]),
                                srgb_to_linear(c[1]),
                                srgb_to_linear(c[2]),
                                m.color[3],
                            ];
                        }
                        if r2.changed() {
                            next.emissive = e.map(srgb_to_linear);
                        }
                        next.metallic = met;
                        next.roughness = rough;
                        send(pending, ModelCommand::Material(i, next));
                    }
                    if r3.drag_stopped() || r4.drag_stopped() {
                        send(pending, ModelCommand::Seal);
                    }
                });
            });
        }
    });
}

/// `extras.ocw`, the copyright and the manifest entry (16B *Metadata*).
fn metadata_section(ui: &mut egui::Ui, form: &mut ModelsForm, d: &ModelDoc, pending: &mut Pending) {
    egui::CollapsingHeader::new("Metadata: extras.ocw and the manifest entry")
        .id_salt("model-edit-meta")
        .default_open(false)
        .show(ui, |ui| {
            ui.strong("extras.ocw");
            if let Some(o) = d.model.ocw() {
                for (k, v) in o {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{k} = {v}"));
                        let fixed = matches!(k.as_str(), "schema" | "kind" | "id");
                        if !fixed && ui.small_button("x").on_hover_text("remove").clicked() {
                            send(pending, ModelCommand::Ocw(k.clone(), None));
                        }
                        if !fixed && ui.small_button("edit").clicked() {
                            form.extras_key.clone_from(k);
                            form.extras_value = v.to_string();
                        }
                    });
                }
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut form.extras_key)
                        .hint_text("key")
                        .desired_width(90.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut form.extras_value)
                        .hint_text("value (JSON or text)")
                        .desired_width(140.0),
                );
                if ui.button("Set").clicked() && !form.extras_key.trim().is_empty() {
                    let v = form.extras_value.trim();
                    let value = serde_json::from_str(v)
                        .unwrap_or_else(|_| serde_json::Value::String(v.into()));
                    send(
                        pending,
                        ModelCommand::Ocw(form.extras_key.trim().into(), Some(value)),
                    );
                    send(pending, ModelCommand::Seal);
                }
            });
            ui.separator();
            ui.strong(format!("Manifest entry ({})", d.entry.file));
            let e = &d.entry;
            for key in ocw_edit::model::ENTRY_KEYS {
                let mut v = match key {
                    "license" => e.license.clone(),
                    "source" => e.source.clone(),
                    "author" => e.author.clone(),
                    "profile" => e.profile.clone().unwrap_or_default(),
                    "species" => e.species.clone().unwrap_or_default(),
                    "variant" => e.variant.clone().unwrap_or_default(),
                    _ => e.anchor.clone().unwrap_or_default(),
                };
                ui.horizontal(|ui| {
                    ui.label(format!("{key:8}"));
                    let r = ui.add(egui::TextEdit::singleline(&mut v).desired_width(240.0));
                    if r.changed() {
                        send(pending, ModelCommand::Entry(key.into(), v.clone()));
                    }
                    if r.lost_focus() {
                        send(pending, ModelCommand::Seal);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("copyright");
                let mut c = d.model.copyright.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut c).desired_width(240.0));
                if r.changed() {
                    send(pending, ModelCommand::Copyright(c));
                }
                if r.lost_focus() {
                    send(pending, ModelCommand::Seal);
                }
            });
        });
}

/// Save to a path, re-import a file, *Open in Blender*.
fn files_section(ui: &mut egui::Ui, form: &mut ModelsForm, studio: &Studio, pending: &mut Pending) {
    egui::CollapsingHeader::new("File: save as, re-import, Blender")
        .id_salt("model-edit-files")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut form.save_as)
                        .hint_text("path.glb")
                        .desired_width(200.0),
                );
                if ui.button("Save as…").clicked() && !form.save_as.trim().is_empty() {
                    send(
                        pending,
                        ModelCommand::Save(Some(crate::edit::expand_home(form.save_as.trim()))),
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut form.reimport)
                        .hint_text("edited .glb (empty: the model's file)")
                        .desired_width(200.0),
                );
                if ui.button("Re-import").clicked() {
                    let p = form.reimport.trim();
                    send(
                        pending,
                        ModelCommand::Reimport(
                            (!p.is_empty()).then(|| crate::edit::expand_home(p)),
                        ),
                    );
                }
            });
            let blender = crate::model_edit::find_blender();
            ui.horizontal(|ui| {
                let label = if blender.is_some() {
                    "Open in Blender"
                } else {
                    "Export editable copy"
                };
                if ui
                    .button(label)
                    .on_hover_text(
                        "Writes an editable copy and re-imports it when it changes (15D)",
                    )
                    .clicked()
                {
                    send(pending, ModelCommand::OpenInBlender);
                }
                if let Some(c) = &studio.models.blender_copy {
                    ui.weak(format!("watching {}", crate::edit::shown_path(c)));
                }
            });
        });
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

fn findings_section(ui: &mut egui::Ui, view: &mut ModelView, editing: bool, pending: &mut Pending) {
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
                let c = if f.severity == Severity::Error {
                    ERROR
                } else {
                    WARNING
                };
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
                        if editing && ui.small_button("Fix").clicked() {
                            send(pending, ModelCommand::Fix(fix.clone()));
                        }
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
            if editing
                && o.findings.iter().any(|f| f.fix.is_some())
                && ui.button("Fix all").clicked()
            {
                send(pending, ModelCommand::FixAll);
            }
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
    let cycles =
        TryOn::default_for(kind, &o.id).is_none() && crate::models_cycle::has_cycles(&o.model);
    egui::CollapsingHeader::new(if cycles { "Flight cycles" } else { "Try it on" })
        .default_open(true)
        .show(ui, |ui| {
            let Some(default) = TryOn::default_for(kind, &o.id) else {
                if crate::models_cycle::has_cycles(&o.model) {
                    crate::models_cycle::section(ui, view);
                } else {
                    ui.weak("No try-on for this kind yet.");
                }
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

fn tree_section(
    ui: &mut egui::Ui,
    view: &mut ModelView,
    doc: Option<&ModelDoc>,
    pending: &mut Pending,
) {
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
                    // The document's selection (the edit gizmo's node).
                    if let Some(d) = doc {
                        let id = if lit { None } else { d.id_of(r.node) };
                        send(pending, ModelCommand::Select(id));
                    }
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

/// The edit gizmo and picking in the 3D view (E4): a drag on a handle
/// moves, turns or scales the selected node (one undo step); a click
/// selects the node under the pointer (its mesh, or a contract point's
/// marker). `true`: the pointer is the gizmo's (the camera does not
/// orbit).
pub fn edit_input(
    ctx: &egui::Context,
    resp: &egui::Response,
    view: &mut ModelView,
    camera: (&Camera, &GlobalTransform),
    studio: &Studio,
    pending: &mut Pending,
) -> bool {
    use crate::model_gizmo::{GizmoDrag, drag, pick_handle, pick_node};
    view.gizmo_mode = studio.models.gizmo;
    let Some(doc) = studio.models.document(studio) else {
        view.gizmo_hot = None;
        return false;
    };
    let (cam, cam_t) = camera;
    let project = |p: Vec3| cam.world_to_viewport(cam_t, p).ok();
    let pointer = resp
        .hover_pos()
        .or_else(|| resp.interact_pointer_pos())
        .map(|p| Vec2::new(p.x, p.y));
    let frame = crate::models_view::edit_frame(view);
    if let Some(g) = view.gizmo_drag {
        if let Some(to) = pointer
            && resp.dragged()
        {
            let t = drag(
                &g.frame,
                g.mode,
                g.axis,
                g.start,
                g.parent_rot,
                cam_t.translation(),
                project,
                g.from,
                to,
                studio.models.snapping || ctx.input(|i| i.modifiers.ctrl),
            );
            send(pending, ModelCommand::Transform(g.node, t));
        }
        if !resp.dragged() || resp.drag_stopped() {
            view.gizmo_drag = None;
            send(pending, ModelCommand::Seal);
        }
        return true;
    }
    view.gizmo_hot = match (frame, pointer) {
        (Some(f), Some(m)) => pick_handle(&f, view.gizmo_mode, project, m),
        _ => None,
    };
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let (Some(axis), Some(f), Some(id)) =
            (view.gizmo_hot, frame, studio.models.selected(studio))
        && let Some(i) = doc.content().index_of(id)
        && let Some(o) = &view.open
    {
        let offset = crate::models_view::main_offset(view).unwrap_or_default();
        let parent = match o.model.nodes[i].parent {
            Some(p) => o.model.world(p),
            None => bevy::math::Affine3A::IDENTITY,
        };
        let (_, parent_rot, _) = (bevy::math::Affine3A::from_mat4(offset.to_matrix()) * parent)
            .to_scale_rotation_translation();
        let from = ctx
            .input(|inp| inp.pointer.press_origin())
            .map(|p| Vec2::new(p.x, p.y))
            .or(pointer)
            .unwrap_or_default();
        view.gizmo_drag = Some(GizmoDrag {
            node: id,
            axis,
            mode: view.gizmo_mode,
            frame: f,
            start: Trs::of(&o.model.nodes[i]),
            parent_rot,
            from,
        });
        return true;
    }
    if view.gizmo_hot.is_some() && resp.is_pointer_button_down_on() {
        return true;
    }
    // A click selects what is under the pointer.
    if resp.clicked()
        && let Some(m) = pointer
        && let Ok(ray) = cam.viewport_to_world(cam_t, m)
        && let Some(o) = &view.open
    {
        let offset = bevy::math::Affine3A::from_mat4(
            crate::models_view::main_offset(view)
                .unwrap_or_default()
                .to_matrix(),
        );
        let level = view.level();
        let shown = |i: usize| {
            let l = models::node_level(&o.model, i);
            l == level
        };
        let points: Vec<(usize, Vec3)> = o
            .gizmos
            .iter()
            .map(|g| (g.node, offset.transform_point3(g.position())))
            .collect();
        let hit = pick_node(
            &o.model,
            offset,
            ray.origin,
            *ray.direction,
            shown,
            &points,
            project,
            m,
        );
        let id = hit.and_then(|i| doc.content().id_of(i));
        send(pending, ModelCommand::Select(id));
    }
    false
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
