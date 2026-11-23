//! The Models workspace's edits (spec §14 Phase 16, 16B, stage E4): the
//! model shown in the viewer (E3) is an `ocw_edit::model::ModelDoc` in the
//! studio's session, so every edit is a command with undo / redo, merged
//! drags, dirty flags and autosave like the World and Routes documents.
//!
//! - **Opening** a model in the viewer (the browser, `--model`, `model
//!   open`) opens its document; every change of the document goes back to
//!   the viewer (a transform alone moves the drawn nodes, anything else
//!   rebuilds the model), with the validator's findings, the stats and the
//!   gizmos of the edited model.
//! - **Commands** ([`ModelCommand`]): select, transform (the gizmo and the
//!   numeric fields), rename to contract names, re-parent, delete; add
//!   anchors, hotspots, levels and collision proxies (a mesh or a scaled
//!   empty, 15H Q9 open); assign paint slots, edit the materials; edit
//!   `extras.ocw`, the copyright and the manifest entry; apply the
//!   validator's fixes; save (the diff first), re-import an edited file,
//!   *Open in Blender*.
//! - **Saving** writes the canonical `.glb` through the Phase 15 writer
//!   and the manifest entry as `model import` does – after the `model
//!   diff` against the file as it was opened (or last saved) has been
//!   shown ([`SavePrompt`], confirmed with *Save*).
//! - **Re-import:** a changed file (an export from Blender, or *Re-import*
//!   with a path) comes back into the document as one undoable step
//!   (`ModelDoc::reimport`: normalised as `model import` does, with what
//!   Blender drops kept); the studio's own save does not count as a change.
//!   *Open in Blender* writes an editable copy to
//!   `<data dir>/ocw/studio/blender/` and starts Blender with it when it is
//!   installed (`OCW_BLENDER`, `blender` on the path, `~/.local/bin`); the
//!   copy is watched and an export over it is re-imported.
//!
//! The console has it all as `studio model …` ([`console_model`]).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use ocw_assets::check::Fix;
use ocw_assets::library::Source;
use ocw_assets::manifest::Entry;
use ocw_assets::model::{Material, Model};
use ocw_edit::model::{ModelDoc, ModelItem, ModelOp, NodeId, Point, ProxyForm, Trs, validator};
use ocw_edit::{DocId, Document};

use crate::edit::Studio;
use crate::model_gizmo::GizmoMode;
use crate::models_view::{Highlight, ModelView};

/// The diff shown before a save, waiting for *Save* or *Cancel*.
#[derive(Debug, Clone, PartialEq)]
pub struct SavePrompt {
    pub path: PathBuf,
    /// The manifest written with it (a model of the folder).
    pub manifest_path: Option<PathBuf>,
    /// `model diff` lines (the totals, then the changes).
    pub lines: Vec<String>,
    /// No structural change (`model diff` is empty).
    pub unchanged: bool,
    /// The contract's errors in the file to be written.
    pub errors: Vec<String>,
    bytes: Vec<u8>,
    manifest: Option<String>,
    /// The document state the prompt was made for.
    state: u64,
}

/// The Models workspace's document and what goes with it.
#[derive(Debug, Default)]
pub struct ModelTools {
    pub doc: Option<DocId>,
    /// The viewer's id of the model the document is for.
    pub opened: Option<String>,
    /// The model and entry as opened or last saved (the diff's left side).
    pub baseline: Option<(Model, Entry)>,
    /// The document state the viewer shows.
    synced: Option<u64>,
    pub prompt: Option<SavePrompt>,
    /// The bytes the studio wrote last (its own save is no outside change).
    saved_bytes: Option<Vec<u8>>,
    /// The editable copy *Open in Blender* wrote, and its last size and
    /// modification time.
    pub blender_copy: Option<PathBuf>,
    blender_seen: Option<(u64, std::time::SystemTime)>,
    /// The viewer's model the document was last shown as (a model opened
    /// anew in the viewer is another one).
    shown: Option<std::sync::Arc<Model>>,
    pub gizmo: GizmoMode,
    pub snapping: bool,
}

impl ModelTools {
    pub fn document<'a>(&self, s: &'a Studio) -> Option<&'a Document<ModelDoc>> {
        s.session.get(self.doc?)
    }

    /// The selected node (its id), if one is selected.
    pub fn selected(&self, s: &Studio) -> Option<NodeId> {
        self.document(s)?.selection.iter().find_map(|i| match i {
            ModelItem::Node(id) => Some(*id),
            ModelItem::Material(_) => None,
        })
    }
}

/// An edit of the Models workspace.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelCommand {
    /// Selects a node (`None`: nothing).
    Select(Option<NodeId>),
    Transform(NodeId, Trs),
    Rename(NodeId, String),
    /// Under a parent (`None`: a root); `keep_place`: stays where it is.
    Reparent(NodeId, Option<NodeId>, bool),
    Delete(NodeId),
    /// A point under the parent at a position in its frame.
    AddPoint(Point, Option<NodeId>, [f32; 3]),
    AddProxy {
        shape: String,
        form: ProxyForm,
        parent: Option<NodeId>,
        centre: [f32; 3],
        half: [f32; 3],
    },
    /// Paint slot to material `index` (with the slot's default colours).
    Slot(usize, String, bool),
    Material(usize, Material),
    /// `extras.ocw.<key>` (`None`: removed).
    Ocw(String, Option<serde_json::Value>),
    Entry(String, String),
    Copyright(String),
    Fix(Fix),
    FixAll,
    /// Shows the diff of a save (to the model's file, or this path).
    Save(Option<PathBuf>),
    ConfirmSave,
    CancelSave,
    /// An edited file into the document (`None`: the model's own file).
    Reimport(Option<PathBuf>),
    OpenInBlender,
    Gizmo(GizmoMode),
    Snapping(bool),
    /// The drag or the typing is over: the next edit is a new undo step.
    Seal,
}

fn now_secs() -> f64 {
    // Only differences matter (the merge window); the studio's clock.
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// Where a save of the open model writes: its file in the model folder
/// (with the manifest), or the file it was opened from; `None` for the
/// embedded models (read-only: save to a path).
pub fn save_target(view: &ModelView) -> Option<(PathBuf, Option<PathBuf>)> {
    let o = view.open.as_ref()?;
    if let Some(p) = &o.path {
        return Some((p.clone(), None));
    }
    match view.lib.source() {
        Source::Folder(dir) if o.listed => Some((
            dir.join(&o.entry.file),
            Some(dir.join(ocw_assets::manifest::MANIFEST)),
        )),
        _ => None,
    }
}

/// Where the model's file is read from for a re-import.
fn own_file(view: &ModelView) -> Option<PathBuf> {
    save_target(view).map(|(p, _)| p)
}

/// Blender: `OCW_BLENDER`, `blender` on the path, `~/.local/bin/blender`.
pub fn find_blender() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OCW_BLENDER") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| {
                d.join(if cfg!(windows) {
                    "blender.exe"
                } else {
                    "blender"
                })
            })
            .find(|p| p.is_file())
    });
    on_path.or_else(|| {
        let p = dirs::home_dir()?.join(".local/bin/blender");
        p.is_file().then_some(p)
    })
}

fn run(studio: &mut Studio, op: ModelOp, now_s: f64, seal: bool) -> Result<(), String> {
    let id = studio.models.doc.ok_or("no model is open")?;
    let doc = studio
        .session
        .get_mut::<ModelDoc>(id)
        .ok_or("no model is open")?;
    if seal {
        doc.seal();
    }
    doc.execute(op, now_s).map_err(|e| e.to_string())?;
    if seal {
        doc.seal();
    }
    Ok(())
}

fn content(studio: &Studio) -> Result<&ModelDoc, String> {
    studio
        .models
        .document(studio)
        .map(Document::content)
        .ok_or_else(|| "no model is open".into())
}

fn node_name(d: &ModelDoc, id: NodeId) -> String {
    d.node(id)
        .map_or_else(|| format!("node {id}"), |n| n.name.clone())
}

/// Carries out a Models command.
pub fn command(
    studio: &mut Studio,
    view: &mut ModelView,
    cmd: ModelCommand,
    now_s: f64,
) -> Result<String, String> {
    match cmd {
        ModelCommand::Select(id) => {
            let doc_id = studio.models.doc.ok_or("no model is open")?;
            let doc = studio
                .session
                .get_mut::<ModelDoc>(doc_id)
                .ok_or("no model is open")?;
            match id {
                Some(id) => {
                    let name = doc
                        .content()
                        .node(id)
                        .map(|n| n.name.clone())
                        .ok_or_else(|| format!("no node {id}"))?;
                    doc.selection.select_only(ModelItem::Node(id));
                    if let Some(i) = doc.content().index_of(id) {
                        view.highlight = Highlight::Node(i);
                        view.style_dirty = true;
                    }
                    Ok(format!("selected {name}"))
                }
                None => {
                    doc.selection.clear();
                    view.highlight = Highlight::None;
                    view.style_dirty = true;
                    Ok("nothing selected".into())
                }
            }
        }
        ModelCommand::Transform(id, trs) => {
            let op = content(studio)?.transform(id, trs);
            run(studio, op, now_s, false)?;
            Ok(String::new())
        }
        ModelCommand::Rename(id, name) => {
            let d = content(studio)?;
            let old = node_name(d, id);
            let op = d.rename(id, &name).map_err(|e| e.to_string())?;
            run(studio, op, now_s, true)?;
            Ok(format!("renamed {old} to {}", name.trim()))
        }
        ModelCommand::Reparent(id, parent, keep) => {
            let d = content(studio)?;
            let msg = format!(
                "{} under {}",
                node_name(d, id),
                parent.map_or_else(|| "the root".into(), |p| node_name(d, p))
            );
            let op = d.reparent(id, parent, keep).map_err(|e| e.to_string())?;
            run(studio, op, now_s, true)?;
            Ok(msg)
        }
        ModelCommand::Delete(id) => {
            let d = content(studio)?;
            let n = d.subtree(id).len();
            let name = node_name(d, id);
            let op = d.delete(id).map_err(|e| e.to_string())?;
            run(studio, op, now_s, true)?;
            Ok(if n > 1 {
                format!("deleted {name} and {} below it", n - 1)
            } else {
                format!("deleted {name}")
            })
        }
        ModelCommand::AddPoint(p, parent, at) => {
            let d = content(studio)?;
            let name = p.name();
            let op = d
                .add_point(&p, parent, Vec3::from(at))
                .map_err(|e| e.to_string())?;
            let new_id = d.fresh_id();
            run(studio, op, now_s, true)?;
            command(studio, view, ModelCommand::Select(Some(new_id)), now_s)?;
            Ok(format!("added {name}"))
        }
        ModelCommand::AddProxy {
            shape,
            form,
            parent,
            centre,
            half,
        } => {
            let d = content(studio)?;
            let op = d
                .add_proxy(&shape, form, parent, Vec3::from(centre), Vec3::from(half))
                .map_err(|e| e.to_string())?;
            let new_id = d.fresh_id();
            run(studio, op, now_s, true)?;
            command(studio, view, ModelCommand::Select(Some(new_id)), now_s)?;
            let name = node_name(content(studio)?, new_id);
            Ok(format!("added {name} ({} form)", form.name()))
        }
        ModelCommand::Slot(i, slot, defaults) => {
            let d = content(studio)?;
            let old = d
                .model
                .materials
                .get(i)
                .map(|m| m.name.clone())
                .ok_or_else(|| format!("no material {i}"))?;
            let op = d
                .assign_slot(i, &slot, defaults)
                .map_err(|e| e.to_string())?;
            let merged = matches!(op, ModelOp::Replace(_));
            run(studio, op, now_s, true)?;
            view.highlight = Highlight::Material(slot.clone());
            view.style_dirty = true;
            Ok(if merged {
                format!("material {old} merged into {slot}")
            } else {
                format!("material {old} is paint slot {slot}")
            })
        }
        ModelCommand::Material(i, m) => {
            let op = content(studio)?
                .set_material(i, |x| *x = m)
                .map_err(|e| e.to_string())?;
            run(studio, op, now_s, false)?;
            Ok(String::new())
        }
        ModelCommand::Ocw(key, value) => {
            let op = content(studio)?
                .set_ocw(&key, value.clone())
                .map_err(|e| e.to_string())?;
            run(studio, op, now_s, false)?;
            Ok(match value {
                Some(v) => format!("extras.ocw.{key} = {v}"),
                None => format!("extras.ocw.{key} removed"),
            })
        }
        ModelCommand::Entry(key, value) => {
            let op = content(studio)?
                .set_entry(&key, &value)
                .map_err(|e| e.to_string())?;
            run(studio, op, now_s, false)?;
            Ok(format!("manifest {key} = \"{}\"", value.trim()))
        }
        ModelCommand::Copyright(text) => {
            run(studio, ModelOp::Copyright(text), now_s, false)?;
            Ok("copyright set".into())
        }
        ModelCommand::Fix(fix) => {
            let op = content(studio)?
                .fix_op(&fix)
                .ok_or_else(|| format!("nothing to do for: {}", fix.describe()))?;
            run(studio, op, now_s, true)?;
            Ok(format!("fixed: {}", fix.describe()))
        }
        ModelCommand::FixAll => {
            let d = content(studio)?;
            let findings = view
                .open
                .as_ref()
                .map(|o| o.findings.clone())
                .unwrap_or_default();
            let op = d.fix_all_op(&findings).ok_or("no finding has a fix")?;
            let label = <ModelDoc as ocw_edit::DocumentKind>::label(&op);
            run(studio, op, now_s, true)?;
            Ok(label.to_lowercase())
        }
        ModelCommand::Save(path) => {
            let (path, manifest_path) = match path {
                Some(p) => {
                    // Into the folder's own file: with the manifest.
                    let own = save_target(view);
                    match own {
                        Some((f, m)) if f == p => (f, m),
                        _ => (p, None),
                    }
                }
                None => save_target(view).ok_or(
                    "the embedded models are read-only: save to a path (`studio model save PATH`)",
                )?,
            };
            let manifest_text = manifest_path
                .as_ref()
                .map(|m| std::fs::read_to_string(m).unwrap_or_default());
            let doc = studio.models.document(studio).ok_or("no model is open")?;
            let before = studio.models.baseline.as_ref().map(|(m, e)| (m, e));
            let plan = doc.content().save_plan(before, manifest_text.as_deref())?;
            let lines = plan.diff.lines();
            let msg = if plan.diff.is_empty() {
                "no structural change – Save or Cancel".to_string()
            } else {
                format!("{} changes – Save or Cancel", plan.diff.changes.len())
            };
            studio.models.prompt = Some(SavePrompt {
                path,
                manifest_path: manifest_path.filter(|_| plan.manifest.is_some()),
                lines,
                unchanged: plan.diff.is_empty(),
                errors: plan.errors,
                bytes: plan.bytes,
                manifest: plan.manifest,
                state: doc.state(),
            });
            Ok(msg)
        }
        ModelCommand::CancelSave => {
            studio.models.prompt = None;
            Ok("not saved".into())
        }
        ModelCommand::ConfirmSave => {
            let p = studio.models.prompt.take().ok_or("nothing to save")?;
            let doc_id = studio.models.doc.ok_or("no model is open")?;
            let doc = studio
                .session
                .get_mut::<ModelDoc>(doc_id)
                .ok_or("no model is open")?;
            if doc.state() != p.state {
                return Err("the model changed since the diff: save again".into());
            }
            if let Some(dir) = p.path.parent()
                && !dir.as_os_str().is_empty()
            {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&p.path, &p.bytes).map_err(|e| format!("{}: {e}", p.path.display()))?;
            if let (Some(mp), Some(text)) = (&p.manifest_path, &p.manifest) {
                std::fs::write(mp, text).map_err(|e| format!("{}: {e}", mp.display()))?;
            }
            doc.mark_saved();
            let saved = ocw_assets::glb::read(&p.bytes).map_err(|e| e.to_string())?;
            studio.models.baseline = Some((saved, doc.content().entry.clone()));
            studio.models.saved_bytes = Some(p.bytes.clone());
            Ok(format!(
                "saved {} ({} KiB{})",
                crate::edit::shown_path(&p.path),
                p.bytes.len() / 1024,
                if p.manifest_path.is_some() {
                    ", manifest entry"
                } else {
                    ""
                }
            ))
        }
        ModelCommand::Reimport(path) => {
            let path = match path {
                Some(p) => p,
                None => own_file(view).ok_or("the model has no file to read again")?,
            };
            reimport(studio, &path, now_s)
        }
        ModelCommand::OpenInBlender => open_in_blender(studio),
        ModelCommand::Gizmo(m) => {
            studio.models.gizmo = m;
            Ok(format!("gizmo: {}", m.name()))
        }
        ModelCommand::Snapping(on) => {
            studio.models.snapping = on;
            Ok(format!("snapping {}", if on { "on" } else { "off" }))
        }
        ModelCommand::Seal => {
            if let Some(d) = studio
                .models
                .doc
                .and_then(|id| studio.session.get_mut::<ModelDoc>(id))
            {
                d.seal();
            }
            Ok(String::new())
        }
    }
}

/// Reads an edited file back into the document (one undoable step).
fn reimport(studio: &mut Studio, path: &Path, now_s: f64) -> Result<String, String> {
    let (m, mut notes) =
        ocw_assets::glb::read_file_lenient(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let d = content(studio)?;
    let (op, more) = d.reimport(m);
    notes.extend(more);
    let same = match &op {
        ModelOp::Replace(next) => **next == *d,
        _ => false,
    };
    if same {
        return Ok(format!(
            "re-imported {}: no change",
            crate::edit::shown_path(path)
        ));
    }
    run(studio, op, now_s, true)?;
    for n in &notes {
        info!("re-import: {n}");
    }
    Ok(format!(
        "re-imported {}{}",
        crate::edit::shown_path(path),
        if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join("; "))
        }
    ))
}

/// The editable copy for Blender (15D `model export`), and Blender
/// started with it.
fn open_in_blender(studio: &mut Studio) -> Result<String, String> {
    let d = content(studio)?;
    let id = d.entry.id().to_owned();
    let dir = crate::edit::autosave_dir()
        .and_then(|a| a.parent().map(|p| p.join("blender")))
        .ok_or("no data directory for the copy")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut written = None;
    for (name, bytes) in ocw_assets::import::export_files(&id, &d.model, false) {
        let p = dir.join(name);
        std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        written = Some(p);
    }
    let copy = written.ok_or("nothing exported")?;
    studio.models.blender_copy = Some(copy.clone());
    studio.models.blender_seen = file_stamp(&copy);
    let Some(blender) = find_blender() else {
        return Ok(format!(
            "editable copy {} (Blender is not installed: open it in any glTF tool; the studio re-imports it when it changes)",
            crate::edit::shown_path(&copy)
        ));
    };
    let expr = format!(
        "import bpy; bpy.ops.wm.read_factory_settings(use_empty=True); bpy.ops.import_scene.gltf(filepath={:?})",
        copy.display().to_string()
    );
    std::process::Command::new(&blender)
        .arg("--python-expr")
        .arg(expr)
        .spawn()
        .map_err(|e| format!("{}: {e}", blender.display()))?;
    Ok(format!(
        "Blender opened {} – export it over the same file (glTF Binary, +Y up); the studio re-imports it",
        crate::edit::shown_path(&copy)
    ))
}

fn file_stamp(p: &Path) -> Option<(u64, std::time::SystemTime)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// Opens the viewer's model as its document when the viewer opened
/// another model (or the same one anew); `models_active`: the Models
/// workspace is shown (the document becomes the active one).
pub fn ensure_doc(studio: &mut Studio, view: &mut ModelView, models_active: bool) {
    let open_id = view.open.as_ref().map(|o| o.id.clone());
    let same_arc = match (&view.open, &studio.models.shown) {
        (Some(o), Some(a)) => std::sync::Arc::ptr_eq(&o.model, a),
        (None, None) => true,
        _ => false,
    };
    if open_id == studio.models.opened && same_arc {
        return;
    }
    if let Some(old) = studio.models.doc.take() {
        studio.session.close(old);
    }
    studio.models.prompt = None;
    studio.models.saved_bytes = None;
    studio.models.blender_copy = None;
    studio.models.opened = open_id;
    studio.models.synced = None;
    studio.models.baseline = None;
    studio.models.shown = view.open.as_ref().map(|o| o.model.clone());
    view.editing = false;
    let Some(o) = &view.open else { return };
    let previous = studio.session.active_id();
    let content = ModelDoc::new(o.entry.clone(), (*o.model).clone(), o.listed);
    let name = o.id.rsplit('/').next().unwrap_or(&o.id).to_owned();
    let source = o.id.clone();
    let id = studio.session.open(|id| {
        Document::new(id, name, content)
            .with_source(source)
            .with_validator(validator())
    });
    // Opening a model makes it the active document only in the Models
    // workspace (undo elsewhere stays with its document).
    if let Some(a) = previous
        && !models_active
    {
        studio.session.set_active(a);
    }
    studio.models.doc = Some(id);
    studio.models.baseline = Some(((*o.model).clone(), o.entry.clone()));
    studio.models.synced = studio.session.get::<ModelDoc>(id).map(Document::state);
    view.editing = true;
}

/// Opens the viewer's model as a document, and shows the document's
/// changes in the viewer (each frame, before the viewer rebuilds).
pub fn sync(
    mut studio: ResMut<Studio>,
    mut view: ResMut<ModelView>,
    mut timer: Local<f32>,
    time: Res<Time>,
    active: Res<crate::models_view::ModelsActive>,
) {
    let studio = &mut *studio;
    let view = &mut *view;
    ensure_doc(studio, view, active.0);
    // In the Models workspace its document is the active one (undo, redo).
    if active.0
        && let Some(id) = studio.models.doc
        && studio.session.active_id() != Some(id)
    {
        studio.session.set_active(id);
    }
    let Some(doc) = studio
        .models
        .doc
        .and_then(|id| studio.session.get::<ModelDoc>(id))
    else {
        return;
    };
    // The document's changes into the viewer.
    if studio.models.synced != Some(doc.state()) {
        studio.models.synced = Some(doc.state());
        let c = doc.content();
        view.refresh(c.model.clone(), c.entry.clone());
        studio.models.shown = view.open.as_ref().map(|o| o.model.clone());
    }
    // The selection's highlight.
    let selected = doc.selection.iter().find_map(|i| match i {
        ModelItem::Node(id) => doc.content().index_of(*id),
        ModelItem::Material(_) => None,
    });
    view.selected = selected;
    if let Some(i) = selected
        && !matches!(view.highlight, Highlight::Node(j) if j == i)
        && !matches!(view.highlight, Highlight::Material(_))
    {
        view.highlight = Highlight::Node(i);
        view.style_dirty = true;
    }
    // A changed file: re-imported (not the studio's own save).
    let mut external: Option<PathBuf> = None;
    if std::mem::take(&mut view.external_change)
        && let Some(p) = own_file(view)
    {
        let bytes = std::fs::read(&p).ok();
        if bytes.is_some() && bytes != studio.models.saved_bytes {
            external = Some(p);
        }
    }
    *timer += time.delta_secs();
    if *timer >= 1.0 {
        *timer = 0.0;
        if let Some(copy) = studio.models.blender_copy.clone() {
            let stamp = file_stamp(&copy);
            if stamp.is_some() && stamp != studio.models.blender_seen {
                studio.models.blender_seen = stamp;
                external = Some(copy);
            }
        }
    }
    if let Some(p) = external {
        let msg = match reimport(studio, &p, now_secs()) {
            Ok(m) => m,
            Err(e) => format!("re-import failed: {e}"),
        };
        info!("{msg}");
        view.status = msg;
    }
}

// ----- the console: `studio model …` -----

fn f(w: Option<&String>, what: &str) -> Result<f32, String> {
    w.ok_or_else(|| format!("{what} is missing"))?
        .parse::<f32>()
        .map_err(|_| format!("{what} must be a number"))
}

fn vec3(words: &[String], from: usize, what: &str) -> Result<[f32; 3], String> {
    Ok([
        f(words.get(from), what)?,
        f(words.get(from + 1), what)?,
        f(words.get(from + 2), what)?,
    ])
}

fn opt_vec3(words: &[String], from: usize, what: &str, or: [f32; 3]) -> Result<[f32; 3], String> {
    if words.len() > from {
        vec3(words, from, what)
    } else {
        Ok(or)
    }
}

fn rest(words: &[String], from: usize) -> String {
    words.get(from..).map(|r| r.join(" ")).unwrap_or_default()
}

/// A node by name (`selected` / `-`: the selection).
fn named(d: &ModelDoc, name: &str, selected: Option<NodeId>) -> Result<NodeId, String> {
    match name {
        "" | "-" | "selected" => selected.ok_or_else(|| "no node selected".into()),
        n => d.by_name(n).ok_or_else(|| format!("no node `{n}`")),
    }
}

fn material_index(d: &ModelDoc, name: &str) -> Result<usize, String> {
    if let Ok(i) = name.parse::<usize>()
        && i < d.model.materials.len()
    {
        return Ok(i);
    }
    d.model
        .material(name)
        .ok_or_else(|| format!("no material `{name}`"))
}

/// What a `studio model …` line does: a command, or an answer.
pub fn console_model(
    words: &[String],
    studio: &Studio,
    view: &ModelView,
) -> Result<crate::edit::Console, String> {
    use crate::edit::Console::{Run, Say};
    let run = |c: ModelCommand| Ok(Run(crate::edit::StudioCommand::Model(c)));
    let w = |i: usize| words.get(i);
    let head = w(0).map(String::as_str).unwrap_or("");
    // What needs no document.
    match head {
        "gizmo" => {
            let m = w(1)
                .and_then(|s| GizmoMode::parse(s))
                .ok_or("gizmo move|rotate|scale")?;
            return run(ModelCommand::Gizmo(m));
        }
        "snap" => {
            return run(ModelCommand::Snapping(
                w(1).map(String::as_str) != Some("off"),
            ));
        }
        "" => {
            return Err("studio model select|move|nudge|rotate|scale|rename|parent|delete|anchor|hotspot|lod|proxy|slot|colour|metallic|roughness|emissive|extras|entry|copyright|fixes|fix|save|reimport|blender|gizmo|snap|info|undo".into());
        }
        _ => {}
    }
    let doc = studio.models.document(studio).ok_or("no model is open")?;
    let d = doc.content();
    let sel = studio.models.selected(studio);
    match head {
        "info" => {
            let mut out = vec![format!(
                "{} ({}): {} nodes, {} materials, undo {}, redo {}{}",
                d.entry.file,
                d.entry.kind.name(),
                d.model.nodes.len(),
                d.model.materials.len(),
                doc.history().undo.len(),
                doc.history().redo.len(),
                if doc.is_dirty() { ", changed" } else { "" }
            )];
            if let Some(id) = sel
                && let Some(n) = d.node(id)
            {
                let (axis, angle) = n.rotation.to_axis_angle();
                out.push(format!(
                    "selected {}: at {:.3} {:.3} {:.3}, turned {:.1}° about {:.2} {:.2} {:.2}, scale {:.3} {:.3} {:.3}, parent {}",
                    n.name,
                    n.translation.x,
                    n.translation.y,
                    n.translation.z,
                    angle.to_degrees(),
                    axis.x,
                    axis.y,
                    axis.z,
                    n.scale.x,
                    n.scale.y,
                    n.scale.z,
                    d.parent_of(id).map_or_else(|| "none".into(), |p| node_name(d, p))
                ));
            }
            out.push(format!(
                "entry: licence {}, source {}, author {}",
                d.entry.license, d.entry.source, d.entry.author
            ));
            out.push(format!(
                "extras.ocw: {}",
                d.model.ocw().map_or_else(
                    || "none".into(),
                    |o| serde_json::Value::Object(o.clone()).to_string()
                )
            ));
            if let Some(p) = &studio.models.prompt {
                out.push(format!(
                    "save to {} waits:",
                    crate::edit::shown_path(&p.path)
                ));
                out.extend(p.lines.iter().map(|l| format!("  {l}")));
            }
            Ok(Say(out.join("\n")))
        }
        "fixes" => {
            let findings = view
                .open
                .as_ref()
                .map(|o| o.findings.clone())
                .unwrap_or_default();
            let lines: Vec<String> = findings
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    Some(format!(
                        "{i}: {} – fix: {}",
                        f.line(),
                        f.fix.as_ref()?.describe()
                    ))
                })
                .collect();
            Ok(Say(if lines.is_empty() {
                "no finding has a fix".into()
            } else {
                lines.join("\n")
            }))
        }
        "fix" => match w(1).map(String::as_str) {
            Some("all") | None => run(ModelCommand::FixAll),
            Some(n) => {
                let i: usize = n
                    .parse()
                    .map_err(|_| "fix N|all (N from `studio model fixes`)")?;
                let fix = view
                    .open
                    .as_ref()
                    .and_then(|o| o.findings.get(i))
                    .and_then(|f| f.fix.clone())
                    .ok_or_else(|| format!("finding {i} has no fix"))?;
                run(ModelCommand::Fix(fix))
            }
        },
        "select" => match rest(words, 1).as_str() {
            "" | "none" => run(ModelCommand::Select(None)),
            n => run(ModelCommand::Select(Some(named(d, n, sel)?))),
        },
        "nudge" => {
            let id = sel.ok_or("select a node first")?;
            let v = vec3(words, 1, "DX DY DZ")?;
            let mut t = Trs::of(d.node(id).ok_or("no node")?);
            t.translation += Vec3::from(v);
            run(ModelCommand::Transform(id, t))
        }
        "move" | "rotate" | "scale" => {
            let id = sel.ok_or("select a node first")?;
            let v = vec3(words, 1, "X Y Z")?;
            let mut t = Trs::of(d.node(id).ok_or("no node")?);
            match head {
                "move" => t.translation = Vec3::from(v),
                "rotate" => {
                    let r = Vec3::from(v) * (std::f32::consts::PI / 180.0);
                    t.rotation = Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z);
                }
                _ => t.scale = Vec3::from(v),
            }
            run(ModelCommand::Transform(id, t))
        }
        "rename" => {
            let id = sel.ok_or("select a node first")?;
            run(ModelCommand::Rename(id, rest(words, 1)))
        }
        "parent" => {
            let id = sel.ok_or("select a node first")?;
            let keep = w(2).map(String::as_str) != Some("local");
            let parent = match w(1).map(String::as_str) {
                None | Some("none") | Some("root") => None,
                Some(n) => Some(named(d, n, None)?),
            };
            run(ModelCommand::Reparent(id, parent, keep))
        }
        "delete" => {
            let id = sel.ok_or("select a node first")?;
            run(ModelCommand::Delete(id))
        }
        "anchor" | "hotspot" | "lod" => {
            let name = w(1).ok_or("a name is missing")?.clone();
            let point = match head {
                "anchor" => Point::Anchor(name),
                "hotspot" => Point::Hotspot(name),
                _ => Point::Level(name),
            };
            let at = opt_vec3(words, 2, "X Y Z", [0.0; 3])?;
            // Under the selected node (the root for a level).
            let parent = if head == "lod" { None } else { sel };
            run(ModelCommand::AddPoint(point, parent, at))
        }
        "proxy" => {
            let shape = w(1).ok_or("proxy box|cylinder|capsule mesh|empty [CX CY CZ HX HY HZ]")?;
            let form = w(2)
                .and_then(|s| ProxyForm::parse(s))
                .ok_or("the form: mesh or empty")?;
            let (centre, half) = if words.len() > 3 {
                (
                    vec3(words, 3, "the centre")?,
                    vec3(words, 6, "the half extents")?,
                )
            } else {
                // Around the selected node's mesh, else the model.
                let m = &d.model;
                let b = sel
                    .and_then(|id| d.index_of(id))
                    .and_then(|i| m.bounds_where(|j| m.is_below(j, i)))
                    .or_else(|| m.near_bounds())
                    .unwrap_or((Vec3::splat(-0.5), Vec3::splat(0.5)));
                (
                    ((b.0 + b.1) * 0.5).to_array(),
                    ((b.1 - b.0) * 0.5).max(Vec3::splat(0.01)).to_array(),
                )
            };
            run(ModelCommand::AddProxy {
                shape: shape.clone(),
                form,
                parent: None,
                centre,
                half,
            })
        }
        "slot" => {
            let i = material_index(d, w(1).ok_or("slot MATERIAL SLOT [defaults]")?)?;
            let slot = w(2).ok_or("the slot is missing")?.clone();
            run(ModelCommand::Slot(
                i,
                slot,
                w(3).map(String::as_str) == Some("defaults"),
            ))
        }
        "colour" | "color" | "emissive" | "metallic" | "roughness" => {
            let i = material_index(d, w(1).ok_or("MATERIAL VALUE…")?)?;
            let mut m = d.model.materials[i].clone();
            match head {
                "metallic" => m.metallic = f(w(2), "metallic")?.clamp(0.0, 1.0),
                "roughness" => m.roughness = f(w(2), "roughness")?.clamp(0.0, 1.0),
                "emissive" => m.emissive = vec3(words, 2, "R G B (linear)")?,
                _ => {
                    let c = vec3(words, 2, "R G B (linear)")?;
                    m.color = [c[0], c[1], c[2], m.color[3]];
                }
            }
            run(ModelCommand::Material(i, m))
        }
        "extras" => {
            let key = w(1).ok_or("extras KEY VALUE|-")?.clone();
            let v = rest(words, 2);
            let value = match v.as_str() {
                "" | "-" => None,
                s => Some(
                    serde_json::from_str(s).unwrap_or_else(|_| serde_json::Value::String(s.into())),
                ),
            };
            run(ModelCommand::Ocw(key, value))
        }
        "entry" => {
            let key =
                w(1).ok_or("entry license|source|author|profile|species|variant|anchor TEXT")?;
            run(ModelCommand::Entry(key.clone(), rest(words, 2)))
        }
        "copyright" => run(ModelCommand::Copyright(rest(words, 1))),
        "save" => match w(1).map(String::as_str) {
            Some("confirm") | Some("yes") => run(ModelCommand::ConfirmSave),
            Some("cancel") | Some("no") => run(ModelCommand::CancelSave),
            None => run(ModelCommand::Save(None)),
            Some(_) => run(ModelCommand::Save(Some(crate::edit::expand_home(&rest(
                words, 1,
            ))))),
        },
        "reimport" => match w(1) {
            None => run(ModelCommand::Reimport(None)),
            Some(_) => run(ModelCommand::Reimport(Some(crate::edit::expand_home(
                &rest(words, 1),
            )))),
        },
        "blender" => run(ModelCommand::OpenInBlender),
        x => Err(format!("studio model: unknown `{x}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{Console, StudioCommand};

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    /// A studio with the viewer's bike open as its document.
    fn setup() -> (Studio, ModelView) {
        let (lib, label) = (
            ocw_assets::library::ModelLibrary::embedded(),
            "embedded".into(),
        );
        let mut view = ModelView::new(lib, label);
        view.open("bikes/bike-road-endurance").unwrap();
        let mut studio = Studio::new(None, None);
        ensure_doc(&mut studio, &mut view, true);
        (studio, view)
    }

    fn console(s: &mut Studio, v: &mut ModelView, line: &str) -> Result<String, String> {
        match console_model(&words(line), s, v)? {
            Console::Say(t) => Ok(t),
            Console::Run(StudioCommand::Model(c)) => {
                let r = command(s, v, c, 0.0);
                // What sync shows.
                let d = s.models.document(s).unwrap();
                let c = d.content().clone();
                v.refresh(c.model, c.entry);
                r
            }
            Console::Run(other) => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_console_edits_the_model_with_undo() {
        let (mut s, mut v) = setup();
        console(&mut s, &mut v, "select saddle").unwrap();
        assert!(matches!(v.highlight, Highlight::Node(_)));
        console(&mut s, &mut v, "move 0 1.05 0.1").unwrap();
        let d = s.models.document(&s).unwrap().content();
        let saddle = d.by_name("saddle").unwrap();
        assert_eq!(
            d.node(saddle).unwrap().translation,
            Vec3::new(0.0, 1.05, 0.1)
        );
        console(&mut s, &mut v, "anchor bottle 0 0.5 0").unwrap();
        assert!(
            v.open
                .as_ref()
                .unwrap()
                .model
                .node("anchor_bottle")
                .is_some()
        );
        console(&mut s, &mut v, "proxy box empty").unwrap();
        console(&mut s, &mut v, "proxy cylinder mesh 0 0.5 0 0.1 0.5 0.1").unwrap();
        let o = v.open.as_ref().unwrap();
        assert!(o.model.node("col_box").is_some() && o.model.node("col_cylinder").is_some());
        assert_eq!(
            o.gizmos
                .iter()
                .filter(|g| g.kind == crate::models::GizmoKind::Collision)
                .count(),
            2,
            "the viewer shows both proxies"
        );
        console(&mut s, &mut v, "entry author Ada Rider").unwrap();
        console(&mut s, &mut v, "extras finish \"matte\"").unwrap();
        let d = s.models.document(&s).unwrap();
        assert_eq!(d.content().entry.author, "Ada Rider");
        assert_eq!(d.content().model.ocw().unwrap()["finish"], "matte");
        assert!(d.is_dirty());
        // Undo it all: the bike as opened.
        let base = s.models.baseline.clone().unwrap();
        while s.session.undo(1.0).unwrap() {}
        let d = s.models.document(&s).unwrap();
        assert_eq!(d.content().model, base.0);
        assert!(!d.is_dirty());
        assert!(
            console(&mut s, &mut v, "rename crank").is_err(),
            "a taken name"
        );
        assert!(console(&mut s, &mut v, "proxy sphere mesh").is_err());
    }

    #[test]
    fn materials_slots_and_fixes_from_the_console() {
        let (mut s, mut v) = setup();
        let first = v.open.as_ref().unwrap().model.materials[0].name.clone();
        console(&mut s, &mut v, &format!("colour {first} 0.2 0.3 0.4")).unwrap();
        console(&mut s, &mut v, &format!("emissive {first} 0.5 0.5 0")).unwrap();
        console(&mut s, &mut v, &format!("roughness {first} 0.25")).unwrap();
        let m = &v.open.as_ref().unwrap().model.materials[0];
        assert_eq!(m.color[..3], [0.2, 0.3, 0.4]);
        assert_eq!(m.emissive, [0.5, 0.5, 0.0]);
        assert_eq!(m.roughness, 0.25);
        // Three edits of one material in a row: one undo step.
        assert_eq!(s.models.document(&s).unwrap().history().undo.len(), 1);
        // A Blender suffix, fixed from the list.
        console(&mut s, &mut v, "select saddle").unwrap();
        console(&mut s, &mut v, "rename saddle.001").unwrap();
        let fixes = console(&mut s, &mut v, "fixes").unwrap();
        assert!(
            fixes.contains("rename node `saddle.001` to `saddle`"),
            "{fixes}"
        );
        console(&mut s, &mut v, "fix all").unwrap();
        assert!(v.open.as_ref().unwrap().model.node("saddle").is_some());
        assert_eq!(
            console(&mut s, &mut v, "fixes").unwrap(),
            "no finding has a fix"
        );
        assert!(console(&mut s, &mut v, "slot 0 no-such-slot").is_err());
    }

    #[test]
    fn a_save_shows_the_diff_first_and_writes_the_canonical_file() {
        let (mut s, mut v) = setup();
        let dir = std::env::temp_dir().join(format!("ocw-studio-save-{}", std::process::id()));
        let path = dir.join("bike-edited.glb");
        // The embedded library is read-only.
        assert!(console(&mut s, &mut v, "save").is_err());
        // No change: an empty diff.
        console(&mut s, &mut v, &format!("save {}", path.display())).unwrap();
        assert!(s.models.prompt.as_ref().unwrap().unchanged);
        console(&mut s, &mut v, "save cancel").unwrap();
        assert!(!path.exists());
        console(&mut s, &mut v, "select saddle").unwrap();
        console(&mut s, &mut v, "rename saddle_test").unwrap();
        console(&mut s, &mut v, &format!("save {}", path.display())).unwrap();
        let p = s.models.prompt.clone().unwrap();
        assert!(
            p.lines.iter().any(|l| l.contains("saddle_test")),
            "{:?}",
            p.lines
        );
        // A change after the diff: the diff is stale.
        console(&mut s, &mut v, "rename saddle_test2").unwrap();
        assert!(console(&mut s, &mut v, "save confirm").is_err());
        console(&mut s, &mut v, &format!("save {}", path.display())).unwrap();
        console(&mut s, &mut v, "save confirm").unwrap();
        let back = ocw_assets::glb::read_file(&path).unwrap();
        assert!(back.node("saddle_test2").is_some());
        assert!(!s.models.document(&s).unwrap().is_dirty());
        // The saved file is the new baseline: saving again shows nothing.
        console(&mut s, &mut v, &format!("save {}", path.display())).unwrap();
        assert!(s.models.prompt.as_ref().unwrap().unchanged);
        // Re-import of an edited file: one undoable step.
        let mut edited = back.clone();
        let i = edited.node("saddle_test2").unwrap();
        edited.nodes[i].translation.y += 0.01;
        let other = dir.join("from-blender.glb");
        std::fs::write(&other, ocw_assets::glb::write(&edited)).unwrap();
        let msg = console(&mut s, &mut v, &format!("reimport {}", other.display())).unwrap();
        assert!(msg.starts_with("re-imported"), "{msg}");
        let y = v.open.as_ref().unwrap().model.nodes[i].translation.y;
        assert!((y - edited.nodes[i].translation.y).abs() < 1e-4);
        s.session.undo(5.0).unwrap();
        let msg = console(&mut s, &mut v, &format!("reimport {}", path.display())).unwrap();
        assert!(msg.ends_with("no change"), "{msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
