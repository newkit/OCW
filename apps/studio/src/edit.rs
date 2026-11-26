//! The studio's side of the document core (spec §14 Phase 16, 16A): the
//! open documents live in an `ocw_edit::Session`; the panels, the menus,
//! the keys and the console only **send commands** ([`StudioCommand`],
//! queued in [`Pending`]) that [`apply`] carries out – the way the game's
//! UI drives `ClientCore`. Undo / redo act on the active document, the
//! autosave writes every changed document to
//! `<data dir>/ocw/studio/autosave/` once it has been left alone for a few
//! seconds (and at quit), and opening a world offers the autosave an
//! earlier session left for it.
//!
//! **World patches (E5 step 2, 16C):** the World document's content is the
//! world patch; the road and height point commands below turn a click or a
//! field in the panels into one patch operation each (undoable), File →
//! Save writes `<package>.ocwpatch` next to the package, Open patch loads
//! one for the open package (a patch for another package is refused), and
//! *Build package* runs `world-builder` with it ([`crate::build`]).

use std::path::PathBuf;

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command, value_parser};
use ocw_edit::world::{
    PackageRef, PatchInfo, PatchTargets, RoadChange, RoadTarget, View, WorldDoc, WorldItem,
    WorldOp, check_patch, check_targets, check_views, op_point_id, op_road_target,
};
use ocw_edit::{AutosaveStore, DocId, Document, Session, Severity, autosave_name};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_sim::god::FlightModel;
use ocw_world::graph::EdgeId;
use ocw_world::patch::{HeightPoint, PatchClass, PatchOneway};
use ocw_worldstore::details::WorldDetails;

use crate::build::{BuildJob, Sources};
use crate::flight::StudioFlight;
use crate::patch::StudioPackage;

/// `<data dir>/ocw/studio/autosave` (`OCW_DATA_DIR` overrides
/// `<data dir>/ocw`, as in the game).
pub fn autosave_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("OCW_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::data_dir()?.join("ocw"),
    };
    Some(base.join("studio").join("autosave"))
}

/// An autosave an earlier session left for the open world.
#[derive(Debug, Clone, PartialEq)]
pub struct Restorable {
    pub name: String,
    pub json: String,
    /// "3 views, 12 steps of history".
    pub summary: String,
}

/// The studio's documents and their autosave.
#[derive(Resource)]
pub struct Studio {
    pub session: Session,
    /// `None`: no data directory (nothing is autosaved).
    pub store: Option<Box<dyn AutosaveStore>>,
    /// Where the store writes, for the status line.
    pub store_dir: Option<PathBuf>,
    /// The World workspace's document.
    pub world_doc: Option<DocId>,
    pub restorable: Option<Restorable>,
    /// The last message (status line).
    pub status: String,
    /// The open package, as the patch tools need it.
    pub package: Option<StudioPackage>,
    /// Where File → Save writes the patch (`None`: not saved yet; the
    /// default is `<package>.ocwpatch` next to the package).
    pub patch_file: Option<PathBuf>,
    /// Road edits act on the selected edge's whole OSM way (the default,
    /// 16H Q18 (a)); `false`: on the selected edge alone.
    pub way_scope: bool,
    /// How the patch next to a package is answered (`--patch`, 16H Q16).
    pub patch_choice: crate::patch_prompt::PatchChoice,
    /// The patch question waiting for an answer (16H Q16 (b)).
    pub patch_question: Option<crate::patch_prompt::PatchQuestion>,
    /// What *Save patch* to the default file minds after the question.
    pub save_guard: Option<crate::patch_prompt::SaveGuard>,
    /// The last *Build package*.
    pub build: Option<BuildJob>,
    /// The Routes workspace: its document, the road graph, the planners.
    pub routes: crate::routes::RouteTools,
    /// The Models workspace's document (E4, [`crate::model_edit`]).
    pub models: crate::model_edit::ModelTools,
}

impl Studio {
    pub fn new(store: Option<Box<dyn AutosaveStore>>, store_dir: Option<PathBuf>) -> Self {
        Self {
            session: Session::default(),
            store,
            store_dir,
            world_doc: None,
            restorable: None,
            status: String::new(),
            package: None,
            patch_file: None,
            way_scope: true,
            patch_choice: Default::default(),
            patch_question: None,
            save_guard: None,
            build: None,
            routes: crate::routes::RouteTools::default(),
            models: crate::model_edit::ModelTools::default(),
        }
    }

    pub fn world(&self) -> Option<&Document<WorldDoc>> {
        self.session.get(self.world_doc?)
    }

    fn world_mut(&mut self) -> Result<&mut Document<WorldDoc>, String> {
        let id = self.world_doc.ok_or("no world is open")?;
        self.session
            .get_mut::<WorldDoc>(id)
            .ok_or_else(|| "no world is open".into())
    }

    /// The selected road edge (its stable id), if one is selected.
    pub fn selected_edge(&self) -> Option<u64> {
        match self.world()?.selection.single()? {
            WorldItem::Edge(e) => Some(*e),
            _ => None,
        }
    }

    /// The selected height point (its id).
    pub fn selected_point(&self) -> Option<String> {
        match self.world()?.selection.single()? {
            WorldItem::HeightPoint(id) => Some(id.clone()),
            _ => None,
        }
    }

    /// What road edits act on: the selected edge, or its OSM way.
    pub fn selected_target(&self) -> Option<RoadTarget> {
        let e = self.selected_edge()?;
        // An edge without an OSM way (made by the builder) is edited alone.
        let way = self
            .package
            .as_ref()
            .and_then(|p| p.edge(e))
            .map(|e| e.osm_way);
        if self.way_scope
            && let Some(way) = way.filter(|w| *w > 0)
        {
            Some(RoadTarget::Way(way))
        } else {
            Some(RoadTarget::Edge(e))
        }
    }

    /// Where Save writes: the file saved to or opened, else the default.
    pub fn save_path(&self) -> Option<PathBuf> {
        self.patch_file
            .clone()
            .or_else(|| self.package.as_ref().map(StudioPackage::default_patch_path))
    }

    /// Opens a World document on the package with `patch` as its start.
    fn new_world_doc(
        &mut self,
        package_ref: PackageRef,
        name: &str,
        source: &str,
        package: &StudioPackage,
        patch: Option<ocw_world::patch::WorldPatch>,
    ) -> DocId {
        let validators = world_validators(Some(package));
        let name = name.to_owned();
        self.session.open(|id| {
            let mut content = WorldDoc::new(package_ref);
            if let Some(p) = patch {
                content.patch = p;
            }
            let mut d = Document::new(id, name, content).with_source(source.to_string());
            for v in validators {
                d = d.with_validator(v);
            }
            d
        })
    }

    /// Answers the patch question (16H Q16 (b)): `load` loads the patch as
    /// the document's start (as one undoable step when the document has
    /// changes already), else the package stays without it. With no
    /// question waiting, `load` loads the patch next to the package now.
    pub fn answer_patch(&mut self, load: bool, now_s: f64) -> Result<String, String> {
        use crate::patch_prompt::{PatchQuestion, SaveGuard};
        let q = match self.patch_question.take() {
            Some(q) => q,
            None if !load => return Err("no patch question is waiting".into()),
            None => {
                let pkg = self.package.as_ref().ok_or("no world is open")?;
                let id = self
                    .world()
                    .ok_or("no world is open")?
                    .content()
                    .package
                    .id
                    .clone();
                PatchQuestion::find(&pkg.file, pkg.manifest.as_deref(), &id)
                    .filter(PatchQuestion::any)
                    .ok_or("there is no patch next to the package")?
            }
        };
        if !load {
            let default = self.package.as_ref().map(StudioPackage::default_patch_path);
            self.save_guard = default.map(SaveGuard::Skipped);
            return Ok(format!(
                "opened without the patch {} ({} changes); File > Open patch or `studio patch load` loads it",
                q.names(),
                q.changes()
            ));
        }
        let patch = q.merged().ok_or("the patch cannot be loaded")?;
        let n = patch.ops.len();
        let fresh = self.world().is_some_and(|d| !d.can_undo() && !d.is_dirty());
        if fresh {
            // The document's start, as before the question: replace the
            // empty document (nothing to autosave or undo).
            let old = self.world_doc.take().ok_or("no world is open")?;
            let (package_ref, name, source) = {
                let d = self
                    .session
                    .get::<WorldDoc>(old)
                    .ok_or("no world is open")?;
                (
                    d.content().package.clone(),
                    d.name.clone(),
                    d.source.clone().unwrap_or_default(),
                )
            };
            let pkg = self.package.take().ok_or("no world is open")?;
            self.session.close(old);
            let id = self.new_world_doc(package_ref, &name, &source, &pkg, Some(patch));
            self.package = Some(pkg);
            self.session.set_active(id);
            self.world_doc = Some(id);
        } else {
            let d = self.world_mut()?;
            d.execute(WorldOp::SetPatch(Box::new(patch)), now_s)
                .map_err(|e| e.to_string())?;
            d.seal();
            d.selection.clear();
        }
        self.patch_file = q.save_file();
        self.save_guard = q.is_stack().then(|| SaveGuard::Stack(q.names()));
        Ok(format!("patch {} loaded ({n} operations)", q.names()))
    }

    /// A height point's position in package metres (x, z).
    pub fn point_xz(&self, h: &HeightPoint) -> Option<[f64; 2]> {
        let [lat, lon] = h.at?;
        Some(self.package.as_ref()?.projection().project(lat, lon))
    }

    /// The World document of a package just opened (see
    /// [`open_world_doc`]); the message about its patch file, if any.
    pub fn open_package(
        &mut self,
        spec: &str,
        source: &str,
        details: &WorldDetails,
        package: StudioPackage,
        now_s: f64,
    ) -> Option<String> {
        if let Some(old) = self.world_doc.take() {
            // Its last changes go to its autosave first.
            if let Some(store) = self.store.as_deref_mut() {
                self.session.autosave(store, now_s, 0.0);
            }
            self.session.close(old);
        }
        let package_ref = PackageRef {
            spec: spec.into(),
            id: details.id.clone(),
            version: details.version,
        };
        // The patch next to the package (16H Q16 (b)): asked about first,
        // unless `--patch load|skip` answered for this run.
        use crate::patch_prompt::{PatchChoice, PatchQuestion, SaveGuard};
        self.patch_question = None;
        self.save_guard = None;
        self.patch_file = None;
        let question = PatchQuestion::find(&package.file, package.manifest.as_deref(), &details.id);
        let mut msg = None;
        let mut patch = None;
        match question {
            Some(q) if !q.any() => {
                let why: Vec<String> = q
                    .files
                    .iter()
                    .filter_map(|f| f.patch.clone().err())
                    .collect();
                msg = Some(format!("patch not loaded: {}", why.join("; ")));
            }
            Some(q) => match self.patch_choice {
                PatchChoice::Ask => {
                    msg = Some(q.console_line());
                    self.patch_question = Some(q);
                    // Not loaded (yet): a save keeps the file there.
                    self.save_guard = Some(SaveGuard::Skipped(package.default_patch_path()));
                }
                PatchChoice::Load => {
                    msg = Some(format!(
                        "patch {} loaded ({} operations; --patch load)",
                        q.names(),
                        q.changes()
                    ));
                    self.patch_file = q.save_file();
                    if q.is_stack() {
                        self.save_guard = Some(SaveGuard::Stack(q.names()));
                    }
                    patch = q.merged();
                }
                PatchChoice::Skip => {
                    msg = Some(format!(
                        "patch {} not loaded (--patch skip); File > Open patch or `studio patch load` loads it",
                        q.names()
                    ));
                    self.save_guard = Some(SaveGuard::Skipped(package.default_patch_path()));
                }
            },
            None => {}
        }
        let id = self.new_world_doc(package_ref, &details.region, source, &package, patch);
        // Routes are edited on this package's roads; one on another
        // package's is closed (its autosave stays).
        self.routes.set_package(&package, &details.id);
        if let Some(old) = self.routes.doc.take() {
            if let Some(store) = self.store.as_deref_mut() {
                self.session.autosave(store, now_s, 0.0);
            }
            self.session.close(old);
        }
        self.session.set_active(id);
        self.package = Some(package);
        self.world_doc = Some(id);
        let record = autosave_name("world", source);
        self.restorable = self
            .store
            .as_ref()
            .and_then(|s| s.read(&record).ok().flatten())
            .and_then(|json| {
                let r: ocw_edit::AutosaveRecord<WorldDoc, WorldOp> =
                    ocw_edit::AutosaveRecord::from_json(&json).ok()?;
                let steps = r.history.undo.len();
                Some(Restorable {
                    name: record.clone(),
                    summary: format!(
                        "{} views, {} patch operations, {steps} steps of history",
                        r.content.views.len(),
                        r.content.patch.ops.len()
                    ),
                    json,
                })
            });
        msg
    }
}

/// A change of a height point in its panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointChange {
    /// The road's height there (m).
    Height(f64),
    /// A change of the built road's height (m).
    Raise(f64),
    /// The influence along the road (m).
    Radius(f64),
}

/// What *Build package* builds.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildRequest {
    /// The basic step's sources (found next to the package or typed).
    pub sources: Sources,
    pub out: PathBuf,
    /// A folder the user picked where the build recipe's sources are
    /// looked for too (16H Q17 (a)).
    pub source_dir: Option<PathBuf>,
}

/// What the UI asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum StudioCommand {
    Undo,
    Redo,
    /// Saves the camera's view (a name, else "View N").
    AddView(Option<String>),
    RenameView(u32, String),
    /// The camera's view replaces the saved one.
    UpdateView(u32),
    DeleteView(u32),
    GoToView(u32),
    /// Ends a merging edit (a name field lost its focus).
    Seal,
    Open(String),
    RestoreAutosave,
    DiscardAutosave,
    /// Flies the camera above a package point (x, z), at its height above
    /// the ground or this one.
    FlyTo([f64; 2], Option<f64>),
    /// Centre the 2D map on a package point at a zoom (metres per point;
    /// screenshots of the map labels).
    MapView([f64; 2], f64),
    /// Switches a layer of the 2D map on, off or (`None`) over (16H Q15).
    MapLayer(crate::map::Layer, Option<bool>),
    /// Fits the 2D map to the package (`true`: with its neighbours).
    MapFit(bool),
    FlightModel(FlightModel),
    /// Selects what is at a package point (x, z): a height point of the
    /// patch, else the nearest road within the tolerance (m); nothing
    /// there clears the selection.
    SelectAt([f64; 2], f64),
    SelectEdge(u64),
    SelectPoint(String),
    /// Selects the target of patch operation `i`.
    SelectOp(usize),
    ClearSelection,
    /// Road edits act on the selected edge's whole OSM way (else on the
    /// edge).
    WayScope(bool),
    /// Changes an attribute of the selected road (one command).
    SetRoad(RoadChange),
    RemoveRoad,
    RestoreRoad,
    /// Places a height point on the selected road, at the road point
    /// nearest to this one (x, z), else in the road's middle.
    AddPoint(Option<[f64; 2]>),
    ChangePoint(String, PointChange),
    /// Moves a height point to the road point nearest to (x, z).
    MovePoint(String, [f64; 2]),
    RemovePoint(String),
    /// Deletes patch operation `i`.
    DeleteOp(usize),
    SetInfo(PatchInfo),
    /// File → Save: the patch to its file (`<package>.ocwpatch` by default).
    SavePatch,
    SavePatchAs(String),
    /// Loads a patch file onto the open package.
    OpenPatch(String),
    /// Answers the patch question (16H Q16 (b)): `true` *Load it*, else
    /// *Open without it* (`studio patch load | skip`).
    PatchAnswer(bool),
    /// Flies the camera to look at the selected road or point.
    Focus,
    /// Builds the patched package with `world-builder`.
    Build(BuildRequest),
    /// A Routes workspace command (16D).
    Route(crate::routes::RouteCommand),
    /// A Models workspace edit (E4).
    Model(crate::model_edit::ModelCommand),
    /// The workspace shown: its document is the one Undo / Redo act on.
    Workspace(crate::ui::Workspace),
    /// The unicorn's flight cycles in the Models viewer (16B, E3).
    Cycle(crate::models_cycle::CycleCommand),
}

/// The commands sent this frame, carried out by [`run_pending`].
#[derive(Resource, Debug, Default)]
pub struct Pending(pub Vec<StudioCommand>);

impl Pending {
    pub fn send(&mut self, c: StudioCommand) {
        self.0.push(c);
    }
}

fn now(world: &World) -> f64 {
    world
        .get_resource::<Time<Real>>()
        .map_or(0.0, |t| t.elapsed_secs_f64())
}

/// The World document's validators: the views, the patch's format and
/// package id, its targets in the package's road graph.
fn world_validators(package: Option<&StudioPackage>) -> Vec<ocw_edit::Validator<WorldDoc>> {
    let targets = package
        .and_then(StudioPackage::graph)
        .map(PatchTargets::of)
        .unwrap_or_default();
    vec![
        Box::new(check_views),
        Box::new(check_patch),
        check_targets(targets),
    ]
}

/// Opens the World workspace's document for a package just shown (the
/// previous one is closed – its changes are in its autosave), loads the
/// patch next to the package if there is one for it, and looks for an
/// autosave of it.
pub fn open_world_doc(
    world: &mut World,
    spec: &str,
    details: &WorldDetails,
    package: StudioPackage,
    now_s: f64,
) {
    let source = std::fs::canonicalize(spec)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| spec.to_string());
    let Some(mut studio) = world.get_resource_mut::<Studio>() else {
        return;
    };
    if let Some(m) = studio.open_package(spec, &source, details, package, now_s) {
        info!("{m}");
        studio.status = m;
    }
}

/// Carries out one command; the status line's message or an error.
pub fn apply(world: &mut World, cmd: StudioCommand) -> Result<String, String> {
    let t = now(world);
    let camera = {
        let f = &world.resource::<StudioFlight>().flight;
        (f.pos, f.view_yaw(), f.view_pitch())
    };
    // Ctrl + S saves the active document: in the Models workspace the
    // model (its diff first).
    let models_active = {
        let s = world.resource::<Studio>();
        s.models.doc.is_some() && s.session.active_id() == s.models.doc
    };
    let cmd = match cmd {
        StudioCommand::SavePatch if models_active => {
            StudioCommand::Model(crate::model_edit::ModelCommand::Save(None))
        }
        c => c,
    };
    match cmd {
        StudioCommand::Model(c) => {
            world.resource_scope(|world, mut view: Mut<crate::models_view::ModelView>| {
                crate::model_edit::command(&mut world.resource_mut::<Studio>(), &mut view, c, t)
            })
        }
        StudioCommand::Open(spec) => crate::world::open(world, spec.trim()),
        StudioCommand::FlyTo(p, height) => {
            let ground = world
                .get_resource::<ocw_render::Ground>()
                .and_then(|g| (g.0)(p[0], p[1]))
                .unwrap_or(0.0);
            let mut f = world.resource_mut::<StudioFlight>();
            let above = height.unwrap_or(f.height_m.max(20.0));
            let (yaw, pitch) = (f.flight.view_yaw(), f.flight.view_pitch());
            f.jump([p[0], ground + above, p[1]], yaw, pitch);
            Ok(format!("flying at {:.0}, {:.0}", p[0], p[1]))
        }
        StudioCommand::MapView(p, m_per_px) => {
            let mut w = world
                .get_resource_mut::<crate::world::OpenWorld>()
                .ok_or("no world open")?;
            let m = m_per_px.clamp(ocw_map::view::MIN_M_PER_PX, ocw_map::view::MAX_M_PER_PX);
            w.map_view = Some(crate::map::MapView {
                centre: p,
                m_per_px: m,
            });
            Ok(format!("map at {:.0}, {:.0}, {m} m per point", p[0], p[1]))
        }
        StudioCommand::MapFit(all) => {
            let mut w = world
                .get_resource_mut::<crate::world::OpenWorld>()
                .ok_or("no world open")?;
            w.map_view = None;
            w.fit_all = all;
            Ok(if all {
                "map: the world and its neighbours".into()
            } else {
                "map: the world".into()
            })
        }
        StudioCommand::MapLayer(l, on) => {
            let photo = world
                .get_resource::<crate::world::OpenWorld>()
                .map(|w| w.aerial.is_some());
            if l == crate::map::Layer::Aerial && photo == Some(false) && on != Some(false) {
                return Err("this package has no aerial photo (section ORTC)".into());
            }
            let mut ui = world
                .get_resource_mut::<crate::ui::UiState>()
                .ok_or("no map")?;
            let v = ui.layers.set(l, on);
            Ok(format!(
                "map: {} {}",
                l.name(),
                if v { "on" } else { "off" }
            ))
        }
        StudioCommand::FlightModel(m) => {
            let mut f = world.resource_mut::<StudioFlight>();
            f.flight.model = m;
            f.flight.stop();
            Ok(format!("flight: {m:?}"))
        }
        StudioCommand::Focus => {
            let target = focus_point(world.resource::<Studio>()).ok_or("nothing selected")?;
            let (pos, yaw, pitch) = crate::flight::look_at(target);
            world.resource_mut::<StudioFlight>().jump(pos, yaw, pitch);
            // The 2D map shows it too, close up.
            if let Some(mut w) = world.get_resource_mut::<crate::world::OpenWorld>() {
                let mut v = w.map_view.unwrap_or_default();
                v.centre = [target[0], target[2]];
                v.m_per_px = v.m_per_px.min(1.5);
                w.map_view = Some(v);
            }
            Ok(format!("looking at {:.0}, {:.0}", target[0], target[2]))
        }
        StudioCommand::Build(req) => start_build(&mut world.resource_mut::<Studio>(), req),
        StudioCommand::PatchAnswer(load) => crate::patch_prompt::answer(world, load),
        StudioCommand::Cycle(c) => crate::models_cycle::command(
            &mut world.resource_mut::<crate::models_view::ModelView>(),
            c,
        ),
        StudioCommand::Route(c) => {
            let fit = matches!(
                c,
                crate::routes::RouteCommand::Import(_)
                    | crate::routes::RouteCommand::ImportPart(_)
                    | crate::routes::RouteCommand::PackageRoute(_)
                    | crate::routes::RouteCommand::Fit
            );
            let msg = crate::routes::command(&mut world.resource_mut::<Studio>(), c, t)?;
            if fit {
                // The map shows the whole route (its view is made now).
                crate::routes::refresh_view(&mut world.resource_mut::<Studio>());
                // (A file waiting for the choice of a part: all its parts.)
                let ext = crate::routes::fit_extent(&world.resource::<Studio>().routes);
                if let (Some((min, max)), Some(mut w)) =
                    (ext, world.get_resource_mut::<crate::world::OpenWorld>())
                {
                    w.map_view = Some(crate::map::MapView {
                        centre: [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5],
                        m_per_px: ((max[0] - min[0]).max(max[1] - min[1]) / 420.0).max(0.5),
                    });
                }
            }
            Ok(msg)
        }
        StudioCommand::GoToView(id) => {
            let view = world
                .resource::<Studio>()
                .world()
                .and_then(|d| d.content().view(id).cloned())
                .ok_or_else(|| format!("no view {id}"))?;
            world
                .resource_mut::<StudioFlight>()
                .jump(view.pos, view.yaw, view.pitch);
            Ok(format!("at view '{}'", view.name))
        }
        other => {
            let mut studio = world.resource_mut::<Studio>();
            edit(&mut studio, other, camera, t)
        }
    }
}

/// The document commands (pure on the [`Studio`]; `camera`: position,
/// yaw, pitch of the 3D view).
pub fn edit(
    studio: &mut Studio,
    cmd: StudioCommand,
    camera: ([f64; 3], f64, f64),
    now_s: f64,
) -> Result<String, String> {
    let doc = studio.world_doc;
    let world_op = |studio: &mut Studio, op: WorldOp| -> Result<(), String> {
        let id = doc.ok_or("no world is open")?;
        studio
            .session
            .execute::<WorldDoc>(id, op, now_s)
            .map_err(|e| e.to_string())
    };
    match cmd {
        StudioCommand::Undo => {
            let label = studio
                .session
                .active()
                .and_then(|d| d.undo_label().map(String::from));
            match studio.session.undo(now_s).map_err(|e| e.to_string())? {
                true => Ok(format!("undone: {}", label.unwrap_or_default())),
                false => Ok("nothing to undo".into()),
            }
        }
        StudioCommand::Redo => {
            let label = studio
                .session
                .active()
                .and_then(|d| d.redo_label().map(String::from));
            match studio.session.redo(now_s).map_err(|e| e.to_string())? {
                true => Ok(format!("redone: {}", label.unwrap_or_default())),
                false => Ok("nothing to redo".into()),
            }
        }
        StudioCommand::AddView(name) => {
            let name = match name.filter(|n| !n.trim().is_empty()) {
                Some(n) => n,
                None => studio
                    .world()
                    .map_or_else(|| "View 1".into(), |d| d.content().new_view_name()),
            };
            let (pos, yaw, pitch) = camera;
            world_op(
                studio,
                WorldOp::add(View {
                    id: 0,
                    name: name.clone(),
                    pos,
                    yaw,
                    pitch,
                }),
            )?;
            if let Some(d) = doc.and_then(|id| studio.session.get_mut::<WorldDoc>(id)) {
                d.seal();
            }
            Ok(format!("view '{name}' saved"))
        }
        StudioCommand::RenameView(id, name) => {
            world_op(studio, WorldOp::Rename { id, name })?;
            Ok(String::new())
        }
        StudioCommand::UpdateView(id) => {
            let (pos, yaw, pitch) = camera;
            world_op(
                studio,
                WorldOp::Move {
                    id,
                    pos,
                    yaw,
                    pitch,
                },
            )?;
            if let Some(d) = doc.and_then(|id| studio.session.get_mut::<WorldDoc>(id)) {
                d.seal();
            }
            Ok("view moved to the camera".into())
        }
        StudioCommand::DeleteView(id) => {
            world_op(studio, WorldOp::Remove(id))?;
            Ok("view deleted".into())
        }
        StudioCommand::Seal => {
            if let Some(d) = doc.and_then(|id| studio.session.get_mut::<WorldDoc>(id)) {
                d.seal();
            }
            Ok(String::new())
        }
        StudioCommand::RestoreAutosave => {
            let r = studio.restorable.take().ok_or("no autosave to restore")?;
            let id = studio
                .session
                .restore::<WorldDoc>(&r.json, world_validators(studio.package.as_ref()))
                .map_err(|e| e.to_string())?;
            let same = match (
                doc.and_then(|d| studio.session.get::<WorldDoc>(d)),
                studio.session.get::<WorldDoc>(id),
            ) {
                (Some(a), Some(b)) => a.content().package.id == b.content().package.id,
                _ => true,
            };
            if let Some(old) = doc {
                studio.session.close(old);
            }
            studio.world_doc = Some(id);
            Ok(if same {
                format!("restored: {}", r.summary)
            } else {
                format!(
                    "restored: {} (saved for another package version)",
                    r.summary
                )
            })
        }
        StudioCommand::DiscardAutosave => {
            let r = studio.restorable.take().ok_or("no autosave to discard")?;
            if let Some(s) = studio.store.as_mut() {
                s.remove(&r.name)?;
            }
            Ok("autosave discarded".into())
        }
        StudioCommand::Route(c) => crate::routes::command(studio, c, now_s),
        StudioCommand::Workspace(w) => {
            let doc = match w {
                crate::ui::Workspace::Routes => studio.routes.doc,
                crate::ui::Workspace::Models => studio.models.doc,
                _ => studio.world_doc,
            };
            if let Some(id) = doc {
                studio.session.set_active(id);
            }
            Ok(String::new())
        }
        StudioCommand::Open(_)
        | StudioCommand::FlyTo(..)
        | StudioCommand::MapView(..)
        | StudioCommand::MapLayer(..)
        | StudioCommand::MapFit(_)
        | StudioCommand::FlightModel(_)
        | StudioCommand::GoToView(_)
        | StudioCommand::Focus
        | StudioCommand::Model(_)
        | StudioCommand::Cycle(_)
        | StudioCommand::Build(_) => Err("not a document command".into()),
        other => patch_command(studio, other, now_s),
    }
}

/// Rounds degrees to 1e-7 (about a centimetre), so patch files stay tidy.
fn round_deg(v: f64) -> f64 {
    (v * 1e7).round() / 1e7
}

/// Rounds a height or a length to centimetres.
fn round_cm(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Executes a patch command (`None`: nothing would change); `seal`: the
/// next edit starts a new undo step (a click; fields that are typed or
/// dragged leave the step open and seal when they lose the focus).
fn run_op(
    studio: &mut Studio,
    op: Option<WorldOp>,
    now_s: f64,
    seal: bool,
    done: impl Into<String>,
) -> Result<String, String> {
    let Some(op) = op else {
        return Ok("no change".into());
    };
    let d = studio.world_mut()?;
    d.execute(op, now_s).map_err(|e| e.to_string())?;
    if seal {
        d.seal();
    }
    Ok(done.into())
}

fn select(studio: &mut Studio, item: Option<WorldItem>) -> Result<(), String> {
    let d = studio.world_mut()?;
    match item {
        Some(i) => d.selection.select_only(i),
        None => d.selection.clear(),
    }
    Ok(())
}

/// The patch file errors that keep it from being saved.
fn patch_errors(studio: &Studio) -> Vec<String> {
    studio
        .world()
        .map(|d| {
            d.findings()
                .iter()
                .filter(|f| {
                    f.severity == Severity::Error
                        && f.target
                            .as_deref()
                            .is_some_and(|t| t == "patch" || t.starts_with("op "))
                })
                .map(|f| f.message.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The road and height point commands, saving and opening the patch.
fn patch_command(studio: &mut Studio, cmd: StudioCommand, now_s: f64) -> Result<String, String> {
    match cmd {
        StudioCommand::SelectAt(at, tol) => {
            let pkg = studio.package.as_ref().ok_or("no world is open")?;
            let doc = studio.world().ok_or("no world is open")?;
            // Height points first: they sit on the roads.
            let point = doc
                .content()
                .height_points()
                .filter_map(|(_, h)| {
                    let p = studio.point_xz(h)?;
                    let d = (p[0] - at[0]).hypot(p[1] - at[1]);
                    (d <= tol).then(|| (d, h.id.clone()))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let pick = pkg
                .graph()
                .and_then(|g| crate::pick::pick_edge(g, at, tol).map(|p| g.edges[p.edge].id.0));
            if let Some((_, id)) = point {
                select(studio, Some(WorldItem::HeightPoint(id.clone())))?;
                return Ok(format!("height point '{id}'"));
            }
            match pick {
                Some(id) => {
                    select(studio, Some(WorldItem::Edge(id)))?;
                    Ok(road_message(studio, id))
                }
                None => {
                    select(studio, None)?;
                    Ok("no road here".into())
                }
            }
        }
        StudioCommand::SelectEdge(id) => {
            let pkg = studio.package.as_ref().ok_or("no world is open")?;
            pkg.edge(id)
                .ok_or_else(|| format!("no edge {id} in the package"))?;
            select(studio, Some(WorldItem::Edge(id)))?;
            Ok(road_message(studio, id))
        }
        StudioCommand::SelectPoint(id) => {
            let doc = studio.world().ok_or("no world is open")?;
            doc.content()
                .height_point(&id)
                .ok_or_else(|| format!("no height point '{id}' in the patch"))?;
            select(studio, Some(WorldItem::HeightPoint(id.clone())))?;
            Ok(format!("height point '{id}'"))
        }
        StudioCommand::SelectOp(i) => {
            let doc = studio.world().ok_or("no world is open")?;
            let op = doc
                .content()
                .patch
                .ops
                .get(i)
                .ok_or_else(|| format!("no patch operation {}", i + 1))?
                .clone();
            if let Some(t) = op_road_target(&op) {
                let pkg = studio.package.as_ref().ok_or("no world is open")?;
                let Some(&first) = pkg.target_edges(t).first() else {
                    select(studio, None)?;
                    return Ok(format!("{} is not in the package", t.describe()));
                };
                let id = pkg.graph().expect("an edge").edges[first].id.0;
                studio.way_scope = matches!(t, RoadTarget::Way(_));
                select(studio, Some(WorldItem::Edge(id)))?;
                return Ok(road_message(studio, id));
            }
            let id = op_point_id(&op).unwrap_or_default().to_string();
            let defined = studio
                .world()
                .is_some_and(|d| d.content().height_point(&id).is_some());
            if defined {
                select(studio, Some(WorldItem::HeightPoint(id.clone())))?;
                Ok(format!("height point '{id}'"))
            } else {
                select(studio, None)?;
                Ok(format!("height point '{id}' is not defined in this patch"))
            }
        }
        StudioCommand::ClearSelection => {
            select(studio, None)?;
            Ok(String::new())
        }
        StudioCommand::WayScope(on) => {
            studio.way_scope = on;
            Ok(if on {
                "road edits act on the whole OSM way".into()
            } else {
                "road edits act on the selected edge".into()
            })
        }
        StudioCommand::SetRoad(change) => {
            let t = studio.selected_target().ok_or("select a road first")?;
            let op = studio
                .world()
                .and_then(|d| d.content().set_road(t, &change));
            // Typed and dragged values merge until the field is left.
            let seal = !matches!(change, RoadChange::Name(_) | RoadChange::Width(_));
            run_op(studio, op, now_s, seal, format!("{} changed", t.describe()))
        }
        StudioCommand::RemoveRoad => {
            let t = studio.selected_target().ok_or("select a road first")?;
            let op = studio.world().and_then(|d| d.content().remove_road(t));
            run_op(
                studio,
                op,
                now_s,
                true,
                format!("{} removed from the ridable roads", t.describe()),
            )
        }
        StudioCommand::RestoreRoad => {
            let t = studio.selected_target().ok_or("select a road first")?;
            let op = studio.world().and_then(|d| d.content().restore_road(t));
            run_op(
                studio,
                op,
                now_s,
                true,
                format!("{} restored", t.describe()),
            )
        }
        StudioCommand::AddPoint(at) => {
            let edge_id = studio.selected_edge().ok_or("select a road first")?;
            let pkg = studio.package.as_ref().ok_or("no world is open")?;
            let edge = pkg.edge(edge_id).ok_or("the road is not in the package")?;
            let (arc, foot) = match at {
                Some(p) => {
                    let (_, arc, foot) = crate::pick::nearest_on_polyline(&edge.points, p);
                    (arc, foot)
                }
                None => {
                    let arc = edge.length_m() * 0.5;
                    let foot = crate::pick::point_at(&edge.points, arc).ok_or("an empty road")?;
                    (arc, foot)
                }
            };
            let (lat, lon) = pkg.projection().unproject(foot);
            let height = edge.height_at(arc).map(|h| (h * 10.0).round() / 10.0);
            let doc = studio.world().ok_or("no world is open")?;
            let id = doc.content().new_point_id();
            let point = HeightPoint {
                id: id.clone(),
                at: Some([round_deg(lat), round_deg(lon)]),
                edge: Some(EdgeId(edge_id)),
                height_m: height,
                raise_m: height.is_none().then_some(0.0),
                radius_m: Some(ocw_world::patch::DEFAULT_RADIUS_M),
                ..Default::default()
            };
            let op = doc.content().add_height_point(point);
            let msg = match height {
                Some(h) => format!("height point '{id}' at {h:.1} m on edge {edge_id}"),
                None => format!("height point '{id}' on edge {edge_id}"),
            };
            run_op(studio, Some(op), now_s, true, msg.clone())?;
            select(studio, Some(WorldItem::HeightPoint(id)))?;
            Ok(msg)
        }
        StudioCommand::ChangePoint(id, change) => {
            let op = studio.world().and_then(|d| {
                d.content().change_height_point(&id, |h| match change {
                    PointChange::Height(v) => {
                        h.height_m = Some(round_cm(v));
                        h.raise_m = None;
                    }
                    PointChange::Raise(v) => {
                        h.raise_m = Some(round_cm(v));
                        h.height_m = None;
                    }
                    PointChange::Radius(v) => h.radius_m = Some(round_cm(v)),
                })
            });
            run_op(studio, op, now_s, false, String::new())
        }
        StudioCommand::MovePoint(id, at) => {
            let pkg = studio.package.as_ref().ok_or("no world is open")?;
            let graph = pkg.graph().ok_or("the package has no road graph")?;
            let doc = studio.world().ok_or("no world is open")?;
            let (_, h) = doc
                .content()
                .height_point(&id)
                .ok_or_else(|| format!("no height point '{id}'"))?;
            let snap = ocw_world::patch::SNAP_M;
            // On its own road if that is near, else the nearest road.
            let own = h.edge.and_then(|e| pkg.edge(e.0)).and_then(|e| {
                let (d, _, foot) = crate::pick::nearest_on_polyline(&e.points, at);
                (d <= snap).then_some((e.id, foot))
            });
            let (edge, foot) = own
                .or_else(|| {
                    crate::pick::pick_edge(graph, at, snap)
                        .map(|p| (graph.edges[p.edge].id, p.foot))
                })
                .ok_or_else(|| format!("no road within {snap} m: a height point sits on a road"))?;
            let (lat, lon) = pkg.projection().unproject(foot);
            let op = doc.content().change_height_point(&id, |h| {
                h.at = Some([round_deg(lat), round_deg(lon)]);
                h.node = None;
                h.edge = Some(edge);
            });
            run_op(
                studio,
                op,
                now_s,
                true,
                format!("height point '{id}' moved"),
            )
        }
        StudioCommand::RemovePoint(id) => {
            let op = studio
                .world()
                .and_then(|d| d.content().remove_height_point(&id));
            if op.is_none() {
                return Err(format!("no height point '{id}' in this patch"));
            }
            run_op(
                studio,
                op,
                now_s,
                true,
                format!("height point '{id}' removed"),
            )
        }
        StudioCommand::DeleteOp(i) => {
            let doc = studio.world().ok_or("no world is open")?;
            let what = doc
                .content()
                .patch
                .ops
                .get(i)
                .map(|o| o.describe())
                .ok_or_else(|| format!("no patch operation {}", i + 1))?;
            let op = WorldOp::RemoveOp {
                index: i,
                label: "Delete patch operation".into(),
            };
            run_op(studio, Some(op), now_s, true, format!("{what} deleted"))
        }
        StudioCommand::SetInfo(info) => {
            let same = studio.world().is_some_and(|d| d.content().info() == info);
            let op = (!same).then_some(WorldOp::SetInfo(info));
            run_op(studio, op, now_s, false, String::new())
        }
        StudioCommand::SavePatch => save_default(studio),
        StudioCommand::PatchAnswer(load) => studio.answer_patch(load, now_s),
        StudioCommand::SavePatchAs(path) => {
            let path = expand_home(path.trim());
            if path.as_os_str().is_empty() {
                return Err("no file name".into());
            }
            save_patch(studio, path)
        }
        StudioCommand::OpenPatch(path) => {
            let path = expand_home(path.trim());
            let doc = studio.world().ok_or("no world is open")?;
            let id = doc.content().package.id.clone();
            let patch = crate::patch::read_patch(&path, &id)?;
            let n = patch.ops.len();
            let d = studio.world_mut()?;
            d.execute(WorldOp::SetPatch(Box::new(patch)), now_s)
                .map_err(|e| e.to_string())?;
            d.seal();
            d.mark_saved();
            d.selection.clear();
            let msg = format!("patch {} opened ({n} operations)", shown_path(&path));
            studio.patch_file = Some(path);
            studio.save_guard = None;
            Ok(msg)
        }
        _ => Err("not a patch command".into()),
    }
}

/// "edge 123 (way 45): Hauptstrasse, residential".
pub fn road_message(studio: &Studio, id: u64) -> String {
    let Some(pkg) = studio.package.as_ref() else {
        return format!("edge {id}");
    };
    let Some(e) = pkg.edge(id) else {
        return format!("edge {id}");
    };
    // How much a road edit changes (16H Q18: the whole way by default).
    let n = pkg.target_edges(RoadTarget::Way(e.osm_way)).len();
    let acts = match studio.selected_target() {
        Some(RoadTarget::Way(_)) if studio.selected_edge() == Some(id) => {
            format!(", edits act on its {n} edge(s)")
        }
        _ => format!(", {n} edge(s)"),
    };
    format!(
        "edge {id} (way {}{acts}): {}{}",
        e.osm_way,
        e.name
            .as_deref()
            .map(|n| format!("{n}, "))
            .unwrap_or_default(),
        class_name(e.class)
    )
}

/// The OSM `highway` name of a road class.
pub fn class_name(c: ocw_world::graph::RoadClass) -> &'static str {
    use ocw_world::graph::RoadClass as C;
    match c {
        C::Trunk => "trunk",
        C::Primary => "primary",
        C::Secondary => "secondary",
        C::Tertiary => "tertiary",
        C::Unclassified => "unclassified",
        C::Residential => "residential",
        C::Service => "service",
        C::Track => "track",
        C::Cycleway => "cycleway",
        C::Path => "path",
    }
}

/// `~/x` → the home directory's `x` (paths typed in the dialogs).
pub fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(p),
    }
}

/// Writes the patch to `path` and marks the document saved (refused while
/// the patch has errors: it would not read back).
fn save_patch(studio: &mut Studio, path: PathBuf) -> Result<String, String> {
    let errors = patch_errors(studio);
    if let Some(first) = errors.first() {
        return Err(format!(
            "not saved: the patch has {} error(s), first: {first}",
            errors.len()
        ));
    }
    let doc = studio.world().ok_or("no world is open")?;
    let patch = doc.content().patch.clone();
    crate::patch::write_patch(&path, &patch)?;
    let d = studio.world_mut()?;
    d.seal();
    d.mark_saved();
    let msg = format!(
        "saved {} ({} operations)",
        shown_path(&path),
        patch.ops.len()
    );
    studio.patch_file = Some(path);
    Ok(msg)
}

/// File → *Save patch* to the patch's file (else `<package>.ocwpatch`):
/// after *Open without it* the patch that lay there is kept as
/// `<package>.ocwpatch.bak` first; a stack loaded as one patch needs *Save
/// patch as* (16H Q16 (b)).
fn save_default(studio: &mut Studio) -> Result<String, String> {
    use crate::patch_prompt::SaveGuard;
    let path = studio.save_path().ok_or("no world is open")?;
    if studio.patch_file.is_none() {
        match studio.save_guard.clone() {
            Some(SaveGuard::Stack(names)) => {
                return Err(format!(
                    "the patch was loaded from the stack {names}: save it as one file with Save patch as (`studio patch save PATH`)"
                ));
            }
            Some(SaveGuard::Skipped(file)) if file == path && file.is_file() => {
                let mut bak = file.as_os_str().to_owned();
                bak.push(".bak");
                let bak = PathBuf::from(bak);
                std::fs::rename(&file, &bak).map_err(|e| format!("{}: {e}", shown_path(&file)))?;
                let msg = save_patch(studio, path)?;
                studio.save_guard = None;
                return Ok(format!(
                    "{msg}; the patch that was there (not loaded) is kept as {}",
                    shown_path(&bak)
                ));
            }
            _ => {}
        }
    }
    save_patch(studio, path)
}

/// *Build package*: saves the patch if it changed, checks the sources and
/// `world-builder`, and starts the build.
pub fn start_build(studio: &mut Studio, req: BuildRequest) -> Result<String, String> {
    if studio.build.as_ref().is_some_and(BuildJob::running) {
        return Err("a build is running".into());
    }
    let planned = |studio: &Studio, patch: &std::path::Path| {
        let pkg = studio.package.as_ref().ok_or("no world is open")?;
        let facts = package_facts(studio).ok_or("no world is open")?;
        crate::build::plan(&pkg.file, pkg.manifest.as_deref(), &facts, &req, patch).map_err(|m| {
            format!(
                "the build needs the package's source data, missing: {}",
                m.join("; ")
            )
        })
    };
    // What it would run (the patch is saved only when it can run).
    planned(studio, &studio.save_path().ok_or("no world is open")?)?;
    let program = crate::build::world_builder().ok_or(
        "world-builder was not found: put it next to ocw-studio, on the PATH, or name it in OCW_WORLD_BUILDER",
    )?;
    let needs_save = studio.patch_file.is_none() || studio.world().is_some_and(|d| d.is_dirty());
    if needs_save {
        save_default(studio)?;
    }
    let patch = studio.patch_file.clone().ok_or("the patch is not saved")?;
    let plan = planned(studio, &patch)?;
    let pkg = studio.package.as_ref().ok_or("no world is open")?;
    if let Some(dir) = req.out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", shown_path(dir)))?;
    }
    // The package's manifest goes along, so the build keeps its id and its
    // credits (spec §13) and counts the version on.
    let side = {
        let mut s = req.out.as_os_str().to_owned();
        s.push(".manifest.json");
        PathBuf::from(s)
    };
    if let Some(m) = &pkg.manifest
        && !side.exists()
    {
        std::fs::copy(m, &side).map_err(|e| format!("{}: {e}", shown_path(&side)))?;
    }
    let mut job = BuildJob::start_chain(&program, &plan.lines, &plan.checks, req.out.clone());
    job.full = plan.full;
    job.note = plan.note.clone();
    info!("build: {}", job.command);
    if let Some(n) = &plan.note {
        info!("build: {n}");
    }
    studio.build = Some(job);
    Ok(if plan.full {
        format!(
            "building {} – the full chain of its recipe: {} …",
            shown_path(&req.out),
            plan.steps()
        )
    } else if let Some(n) = plan.note {
        format!("building {} – {n}", shown_path(&req.out))
    } else {
        format!("building {} …", shown_path(&req.out))
    })
}

/// What a rebuild keeps of the open package (its name, id, origin, zone).
pub fn package_facts(studio: &Studio) -> Option<crate::build::PackageFacts> {
    let pkg = studio.package.as_ref()?;
    let doc = studio.world()?;
    Some(crate::build::PackageFacts {
        name: pkg.pkg.name.clone(),
        id: doc.content().package.id.clone(),
        origin: pkg.pkg.origin,
        time_zone: pkg.pkg.time_zone.clone(),
    })
}

/// The point the camera looks at for the selection: a height point, else
/// the middle of the selected road (x, height, z).
pub fn focus_point(studio: &Studio) -> Option<[f64; 3]> {
    let pkg = studio.package.as_ref()?;
    if let Some(id) = studio.selected_point() {
        let doc = studio.world()?;
        let (_, h) = doc.content().height_point(&id)?;
        let p = studio.point_xz(h)?;
        let y = h
            .edge
            .and_then(|e| pkg.edge(e.0))
            .and_then(|e| {
                let (_, arc, _) = crate::pick::nearest_on_polyline(&e.points, p);
                e.height_at(arc)
            })
            .or_else(|| pkg.pkg.surface_height(p[0], p[1]))
            .unwrap_or(0.0);
        return Some([p[0], y, p[1]]);
    }
    let e = pkg.edge(studio.selected_edge()?)?;
    let arc = e.length_m() * 0.5;
    let p = crate::pick::point_at(&e.points, arc)?;
    let y = e
        .height_at(arc)
        .or_else(|| pkg.pkg.surface_height(p[0], p[1]))
        .unwrap_or(0.0);
    Some([p[0], y, p[1]])
}

/// Carries out the commands sent this frame.
fn run_pending(world: &mut World) {
    let cmds = std::mem::take(&mut world.resource_mut::<Pending>().0);
    for c in cmds {
        // The Models workspace has its own status line.
        let model = matches!(c, StudioCommand::Model(_)) || {
            let s = world.resource::<Studio>();
            s.models.doc.is_some() && s.session.active_id() == s.models.doc
        };
        let msg = match apply(world, c) {
            Ok(m) => m,
            Err(e) => format!("error: {e}"),
        };
        if !msg.is_empty() {
            info!("{msg}");
            if model && let Some(mut v) = world.get_resource_mut::<crate::models_view::ModelView>()
            {
                v.status = msg.clone();
            }
            world.resource_mut::<Studio>().status = msg;
        }
    }
}

/// Writes the autosaves that are due; at quit all changed documents.
fn autosave(mut studio: ResMut<Studio>, time: Res<Time<Real>>, mut exits: MessageReader<AppExit>) {
    let quitting = exits.read().count() > 0;
    let delay = if quitting {
        0.0
    } else {
        ocw_edit::autosave::DEFAULT_DELAY_S
    };
    let studio = &mut *studio;
    let Some(store) = studio.store.as_deref_mut() else {
        return;
    };
    let (written, errors) = studio
        .session
        .autosave(store, time.elapsed_secs_f64(), delay);
    for e in &errors {
        warn!("autosave: {e}");
    }
    if let Some(e) = errors.first() {
        studio.status = format!("autosave failed: {e}");
    } else if !written.is_empty() {
        info!("autosaved {}", written.join(", "));
    }
}

/// Ctrl + Z, Ctrl + Shift + Z, Ctrl + Y and Ctrl + S (not while a text field has
/// the keyboard: it undoes its own typing).
fn shortcut_keys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    focus: Res<crate::flight::UiFocus>,
    mut pending: ResMut<Pending>,
) {
    let Some(k) = keys else {
        return;
    };
    if focus.keyboard {
        return;
    }
    let ctrl = k.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = k.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if ctrl && k.just_pressed(KeyCode::KeyZ) {
        pending.send(if shift {
            StudioCommand::Redo
        } else {
            StudioCommand::Undo
        });
    } else if ctrl && k.just_pressed(KeyCode::KeyY) {
        pending.send(StudioCommand::Redo);
    } else if ctrl && k.just_pressed(KeyCode::KeyS) {
        pending.send(StudioCommand::SavePatch);
    } else if !ctrl && k.just_pressed(KeyCode::KeyO) {
        // The map's aerial photo (16H Q15).
        pending.send(StudioCommand::MapLayer(crate::map::Layer::Aerial, None));
    }
}

pub struct EditPlugin;

impl Plugin for EditPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pending>()
            .add_systems(Update, (shortcut_keys, run_pending).chain())
            .add_systems(Last, autosave);
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("studio")
                .about("OCW Studio: undo, redo, views, open, fly, world patch editing")
                .subcommand_required(true)
                .subcommand(Command::new("undo").about("Undo the last edit of the active document"))
                .subcommand(Command::new("redo").about("Redo the last undone edit"))
                .subcommand(
                    Command::new("status").about("The documents, their history and autosave"),
                )
                .subcommand(
                    Command::new("open")
                        .about("Open a world package")
                        .arg(Arg::new("path").required(true)),
                )
                .subcommand(
                    Command::new("map")
                        .about("Centre the 2D map on a package point at a zoom (m per point)")
                        .arg(
                            Arg::new("x")
                                .required(true)
                                .allow_hyphen_values(true)
                                .value_parser(value_parser!(f64)),
                        )
                        .arg(
                            Arg::new("z")
                                .required(true)
                                .allow_hyphen_values(true)
                                .value_parser(value_parser!(f64)),
                        )
                        .arg(
                            Arg::new("m_per_px")
                                .required(true)
                                .value_parser(value_parser!(f64)),
                        ),
                )
                .subcommand(
                    Command::new("map-fit")
                        .about("Fit the 2D map to the package (all: with its neighbours)")
                        .arg(Arg::new("all").value_parser(["all"])),
                )
                .subcommand(
                    Command::new("layer")
                        .about(
                            "The 2D map's layers: studio layer [aerial|neighbours|border] [on|off|toggle]",
                        )
                        .arg(Arg::new("layer"))
                        .arg(Arg::new("state")),
                )
                .subcommand(
                    Command::new("fly")
                        .about("Fly the camera above a package point")
                        .arg(
                            Arg::new("x")
                                .required(true)
                                .allow_hyphen_values(true)
                                .value_parser(value_parser!(f64)),
                        )
                        .arg(
                            Arg::new("z")
                                .required(true)
                                .allow_hyphen_values(true)
                                .value_parser(value_parser!(f64)),
                        )
                        .arg(Arg::new("height").value_parser(value_parser!(f64))),
                )
                .subcommand(
                    Command::new("view")
                        .about("Saved views of the World workspace")
                        .subcommand_required(true)
                        .subcommand(Command::new("add").arg(Arg::new("name").num_args(0..)))
                        .subcommand(Command::new("list"))
                        .subcommand(
                            Command::new("go").arg(
                                Arg::new("id")
                                    .required(true)
                                    .value_parser(value_parser!(u32)),
                            ),
                        )
                        .subcommand(
                            Command::new("delete").arg(
                                Arg::new("id")
                                    .required(true)
                                    .value_parser(value_parser!(u32)),
                            ),
                        ),
                )
                .subcommand(words(
                    "select",
                    "Select: X Z (package metres) | latlon LAT LON | edge ID | point ID | op N | none",
                ))
                .subcommand(words(
                    "road",
                    "The selected road: surface V | width M | class C | oneway no|forward|backward | bike closed|open | name TEXT (each `-`: back to the package) | remove | restore | scope edge|way | focus | info",
                ))
                .subcommand(words(
                    "point",
                    "Height points: add [X Z] | height M | raise M | radius M | move X Z | remove | focus (on the selected point)",
                ))
                .subcommand(words(
                    "route",
                    "The Routes workspace: new [NAME] | import PATH | package N | plan X Z X Z … | via add|insert|move|remove … | drag GX GZ X Z | reverse | loop on|off | outback | cut start|end N | split N | profile P | planner builtin|brouter | brouter url URL | brouter profile P | brouter ok | brouter plan | export PATH | save [PATH] | testride | info",
                ))
                .subcommand(words(
                    "model",
                    "The Models workspace's edits (E4): select NAME|none | move|rotate|scale X Y Z | rename NAME | parent NAME|none [keep|local] | delete | anchor NAME [X Y Z] | hotspot SLOT [X Y Z] | lod lod1|lod2|billboard | proxy box|cylinder|capsule mesh|empty [CX CY CZ HX HY HZ] | slot MATERIAL SLOT [defaults] | colour|emissive MATERIAL R G B | metallic|roughness MATERIAL V | extras KEY VALUE|- | entry KEY TEXT | copyright TEXT | fixes | fix N|all | save [PATH] | save confirm|cancel | reimport [PATH] | blender | gizmo move|rotate|scale | snap on|off | info",
                    ))
                .subcommand(words(
                    "cycle",
                    "The unicorn's flight cycles in the Models viewer: wing | gallop | flight | landing | rest | play | pause | speed X | at SECONDS | record DIR [SECONDS] [FPS] | info",
                ))
                .subcommand(words(
                    "patch",
                    "The world patch: list | save [PATH] | open PATH | load | skip (the question when a package with a patch opens) | delete N | title TEXT | author TEXT | build [OUT]",
                )),
            studio_command,
        ));
    }
}

/// A console subcommand that takes free words (parsed by
/// [`console_patch`]).
fn words(name: &'static str, about: &'static str) -> Command {
    Command::new(name).about(about).arg(
        Arg::new("words")
            .num_args(0..)
            .allow_hyphen_values(true)
            .trailing_var_arg(true)
            .action(ArgAction::Append),
    )
}

/// What a patch console command does: a command, or an answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Console {
    Run(StudioCommand),
    Say(String),
}

fn num(w: Option<&String>, what: &str) -> Result<f64, String> {
    w.ok_or_else(|| format!("{what} is missing"))?
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{what} must be a number"))
}

fn rest(words: &[String], from: usize) -> Option<String> {
    let t = words.get(from..).map(|w| w.join(" ")).unwrap_or_default();
    (!t.is_empty()).then_some(t)
}

fn reset(v: &str) -> bool {
    v == "-" || v == "package"
}

/// The `studio select | road | point | patch …` console commands (pure:
/// the words to a command).
pub fn console_patch(group: &str, words: &[String], studio: &Studio) -> Result<Console, String> {
    let w0 = words.first().map(String::as_str).unwrap_or("");
    let run = |c| Ok(Console::Run(c));
    let point = || {
        studio
            .selected_point()
            .ok_or_else(|| "select a height point first".to_string())
    };
    match (group, w0) {
        ("select", "latlon") => {
            let lat = num(words.get(1), "LAT")?;
            let lon = num(words.get(2), "LON")?;
            let pkg = studio.package.as_ref().ok_or("no world is open")?;
            run(StudioCommand::SelectAt(
                pkg.projection().project(lat, lon),
                50.0,
            ))
        }
        ("select", "edge") => run(StudioCommand::SelectEdge(
            words
                .get(1)
                .and_then(|w| w.parse().ok())
                .ok_or("an edge id")?,
        )),
        ("select", "point") => run(StudioCommand::SelectPoint(
            rest(words, 1).ok_or("a height point id")?,
        )),
        ("select", "op") => {
            let n = num(words.get(1), "N")? as usize;
            run(StudioCommand::SelectOp(n.max(1) - 1))
        }
        ("select", "none") => run(StudioCommand::ClearSelection),
        ("select", _) => {
            let x = num(words.first(), "X")?;
            let z = num(words.get(1), "Z")?;
            run(StudioCommand::SelectAt([x, z], 50.0))
        }
        ("road", "remove") => run(StudioCommand::RemoveRoad),
        ("road", "restore") => run(StudioCommand::RestoreRoad),
        ("road", "focus") => run(StudioCommand::Focus),
        ("road", "scope") => match words.get(1).map(String::as_str) {
            Some("way") => run(StudioCommand::WayScope(true)),
            Some("edge") => run(StudioCommand::WayScope(false)),
            _ => Err("studio road scope edge|way".into()),
        },
        ("road", "info") => {
            let id = studio.selected_edge().ok_or("select a road first")?;
            Ok(Console::Say(road_message(studio, id)))
        }
        ("road", attr) if !attr.is_empty() => {
            let v = rest(words, 1).ok_or_else(|| format!("studio road {attr} VALUE"))?;
            let change = match attr {
                "surface" => RoadChange::Surface((!reset(&v)).then_some(v)),
                "width" => RoadChange::Width(if reset(&v) {
                    None
                } else {
                    Some(num(Some(&v), "the width")?)
                }),
                "class" => RoadChange::Class(if reset(&v) {
                    None
                } else {
                    Some(
                        PATCH_CLASSES
                            .iter()
                            .copied()
                            .find(|c| c.osm() == v)
                            .ok_or_else(|| format!("unknown class `{v}`"))?,
                    )
                }),
                "oneway" => RoadChange::Oneway(match v.as_str() {
                    "-" | "package" => None,
                    "no" => Some(PatchOneway::No),
                    "forward" => Some(PatchOneway::Forward),
                    "backward" => Some(PatchOneway::Backward),
                    _ => return Err("oneway no|forward|backward|-".into()),
                }),
                "bike" => RoadChange::BikeClosed(match v.as_str() {
                    "closed" => true,
                    "open" | "-" => false,
                    _ => return Err("bike closed|open".into()),
                }),
                "name" => RoadChange::Name(
                    (!reset(&v)).then(|| if v == "\"\"" { String::new() } else { v }),
                ),
                _ => return Err(format!("no road attribute `{attr}`")),
            };
            run(StudioCommand::SetRoad(change))
        }
        ("point", "add") => {
            let at = match (words.get(1), words.get(2)) {
                (Some(_), Some(_)) => Some([num(words.get(1), "X")?, num(words.get(2), "Z")?]),
                _ => None,
            };
            run(StudioCommand::AddPoint(at))
        }
        ("point", "height") => run(StudioCommand::ChangePoint(
            point()?,
            PointChange::Height(num(words.get(1), "the height")?),
        )),
        ("point", "raise") => run(StudioCommand::ChangePoint(
            point()?,
            PointChange::Raise(num(words.get(1), "the change")?),
        )),
        ("point", "radius") => run(StudioCommand::ChangePoint(
            point()?,
            PointChange::Radius(num(words.get(1), "the radius")?),
        )),
        ("point", "move") => run(StudioCommand::MovePoint(
            point()?,
            [num(words.get(1), "X")?, num(words.get(2), "Z")?],
        )),
        ("point", "remove") => run(StudioCommand::RemovePoint(point()?)),
        ("point", "focus") => run(StudioCommand::Focus),
        ("patch", "list") => Ok(Console::Say(patch_lines(studio).join("\n"))),
        ("patch", "save") => match rest(words, 1) {
            Some(path) => run(StudioCommand::SavePatchAs(path)),
            None => run(StudioCommand::SavePatch),
        },
        ("patch", "load") => run(StudioCommand::PatchAnswer(true)),
        ("patch", "skip") => run(StudioCommand::PatchAnswer(false)),
        ("patch", "open") => run(StudioCommand::OpenPatch(
            rest(words, 1).ok_or("studio patch open PATH")?,
        )),
        ("patch", "delete") => {
            let n = num(words.get(1), "N")? as usize;
            run(StudioCommand::DeleteOp(n.max(1) - 1))
        }
        ("patch", "title" | "author") => {
            let doc = studio.world().ok_or("no world is open")?;
            let mut info = doc.content().info();
            let v = rest(words, 1);
            if w0 == "title" {
                info.title = v;
            } else {
                info.author = v;
            }
            run(StudioCommand::SetInfo(info))
        }
        ("patch", "build") => {
            let pkg = studio.package.as_ref().ok_or("no world is open")?;
            let out = rest(words, 1)
                .map(|p| expand_home(&p))
                .unwrap_or_else(|| crate::build::default_output(&pkg.file));
            run(StudioCommand::Build(BuildRequest {
                sources: crate::build::find_sources(&pkg.file),
                out,
                source_dir: None,
            }))
        }
        _ => Err(format!("studio {group}: see `help studio`")),
    }
}

/// The road classes of the patch format, in the order of the panel.
pub const PATCH_CLASSES: [PatchClass; 10] = [
    PatchClass::Trunk,
    PatchClass::Primary,
    PatchClass::Secondary,
    PatchClass::Tertiary,
    PatchClass::Unclassified,
    PatchClass::Residential,
    PatchClass::Service,
    PatchClass::Track,
    PatchClass::Cycleway,
    PatchClass::Path,
];

/// `studio patch list`: one line per operation, with the preview's
/// status when there is one.
pub fn patch_lines(studio: &Studio) -> Vec<String> {
    let Some(doc) = studio.world() else {
        return vec!["no world is open".into()];
    };
    let p = &doc.content().patch;
    let mut out = vec![format!(
        "patch for {}: {} operations{}",
        p.package,
        p.ops.len(),
        if doc.is_dirty() { " (changed)" } else { "" }
    )];
    for (i, op) in p.ops.iter().enumerate() {
        out.push(format!("  {:>3} {}", i + 1, op.describe()));
    }
    out
}

fn studio_command(world: &mut World, m: &clap::ArgMatches) -> Result<String, String> {
    if let Some(("cycle", a)) = m.subcommand() {
        let words: Vec<String> = a
            .get_many::<String>("words")
            .map(|w| w.cloned().collect())
            .unwrap_or_default();
        return apply(
            world,
            StudioCommand::Cycle(crate::models_cycle::parse(&words)?),
        );
    }
    if let Some(("route", a)) = m.subcommand() {
        let words: Vec<String> = a
            .get_many::<String>("words")
            .map(|w| w.cloned().collect())
            .unwrap_or_default();
        if words.first().map(String::as_str) == Some("info") {
            return Ok(crate::routes::route_lines(world.resource::<Studio>()).join("\n"));
        }
        let cmd = crate::routes::console_route(&words)?;
        let msg = apply(world, StudioCommand::Route(cmd))?;
        if !msg.is_empty() {
            world.resource_mut::<Studio>().status = msg.clone();
        }
        return Ok(msg);
    }
    if let Some(("model", a)) = m.subcommand() {
        let words: Vec<String> = a
            .get_many::<String>("words")
            .map(|w| w.cloned().collect())
            .unwrap_or_default();
        // The viewer's model as a document now (a `--cmd` line may come
        // before the first frame's sync).
        let models_active = world
            .get_resource::<crate::models_view::ModelsActive>()
            .is_some_and(|a| a.0);
        world.resource_scope(|world, mut view: Mut<crate::models_view::ModelView>| {
            crate::model_edit::ensure_doc(
                &mut world.resource_mut::<Studio>(),
                &mut view,
                models_active,
            );
        });
        // The panel's paint slot picker (shown for a material).
        if words.first().map(String::as_str) == Some("picker") {
            let pick = match words.get(1).map(String::as_str) {
                None | Some("off") => None,
                Some(m) => {
                    let s = world.resource::<Studio>();
                    let d = s.models.document(s).ok_or("no model is open")?;
                    let mats = &d.content().model.materials;
                    Some(
                        m.parse::<usize>()
                            .ok()
                            .filter(|i| *i < mats.len())
                            .or_else(|| d.content().model.material(m))
                            .ok_or_else(|| format!("no material `{m}`"))?,
                    )
                }
            };
            world
                .resource_mut::<crate::models_ui::ModelsForm>()
                .slot_picker = pick;
            return Ok(match pick {
                Some(i) => format!("paint slot picker for material {i}"),
                None => "paint slot picker closed".into(),
            });
        }
        let cmd = match crate::model_edit::console_model(
            &words,
            world.resource::<Studio>(),
            world.resource::<crate::models_view::ModelView>(),
        )? {
            Console::Say(text) => return Ok(text),
            Console::Run(c) => c,
        };
        let msg = apply(world, cmd)?;
        if !msg.is_empty() {
            world.resource_mut::<Studio>().status = msg.clone();
            world.resource_mut::<crate::models_view::ModelView>().status = msg.clone();
        }
        return Ok(msg);
    }
    if let Some((group @ ("select" | "road" | "point" | "patch"), a)) = m.subcommand() {
        let words: Vec<String> = a
            .get_many::<String>("words")
            .map(|w| w.cloned().collect())
            .unwrap_or_default();
        let cmd = match console_patch(group, &words, world.resource::<Studio>())? {
            Console::Say(text) => return Ok(text),
            Console::Run(c) => c,
        };
        let msg = apply(world, cmd)?;
        if !msg.is_empty() {
            world.resource_mut::<Studio>().status = msg.clone();
        }
        return Ok(msg);
    }
    let cmd = match m.subcommand() {
        Some(("undo", _)) => StudioCommand::Undo,
        Some(("redo", _)) => StudioCommand::Redo,
        Some(("status", _)) => return Ok(status_lines(world.resource::<Studio>()).join("\n")),
        Some(("open", a)) => {
            StudioCommand::Open(a.get_one::<String>("path").cloned().unwrap_or_default())
        }
        Some(("map", a)) => StudioCommand::MapView(
            [
                *a.get_one::<f64>("x").expect("required"),
                *a.get_one::<f64>("z").expect("required"),
            ],
            *a.get_one::<f64>("m_per_px").expect("required"),
        ),
        Some(("map-fit", a)) => StudioCommand::MapFit(a.get_one::<String>("all").is_some()),
        Some(("layer", a)) => {
            let Some(name) = a.get_one::<String>("layer") else {
                return Ok(format!(
                    "map: {}",
                    world
                        .get_resource::<crate::ui::UiState>()
                        .map(|u| u.layers)
                        .unwrap_or_default()
                        .describe()
                ));
            };
            let l = crate::map::Layer::parse(name)
                .ok_or("studio layer aerial|neighbours|border [on|off|toggle]")?;
            let on = match a.get_one::<String>("state").map(String::as_str) {
                None | Some("toggle") => None,
                Some("on") => Some(true),
                Some("off") => Some(false),
                Some(o) => return Err(format!("{o}: on, off or toggle")),
            };
            StudioCommand::MapLayer(l, on)
        }
        Some(("fly", a)) => StudioCommand::FlyTo(
            [
                *a.get_one::<f64>("x").expect("required"),
                *a.get_one::<f64>("z").expect("required"),
            ],
            a.get_one::<f64>("height").copied(),
        ),
        Some(("view", v)) => match v.subcommand() {
            Some(("add", a)) => {
                let name: Vec<String> = a
                    .get_many::<String>("name")
                    .map(|n| n.cloned().collect())
                    .unwrap_or_default();
                StudioCommand::AddView((!name.is_empty()).then(|| name.join(" ")))
            }
            Some(("list", _)) => {
                let s = world.resource::<Studio>();
                let doc = s.world().ok_or("no world is open")?;
                return Ok(doc
                    .content()
                    .views
                    .iter()
                    .map(|v| {
                        format!(
                            "{} '{}' at {:.0}, {:.0}, {:.0}",
                            v.id, v.name, v.pos[0], v.pos[1], v.pos[2]
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"));
            }
            Some(("go", a)) => StudioCommand::GoToView(*a.get_one::<u32>("id").expect("required")),
            Some(("delete", a)) => {
                StudioCommand::DeleteView(*a.get_one::<u32>("id").expect("required"))
            }
            _ => return Err("studio view add|list|go|delete".into()),
        },
        _ => {
            return Err(
                "studio undo|redo|status|open|fly|view|select|road|point|patch|route|model".into(),
            );
        }
    };
    let msg = apply(world, cmd)?;
    if !msg.is_empty() {
        world.resource_mut::<Studio>().status = msg.clone();
    }
    Ok(msg)
}

/// `studio status`: one line per document.
pub fn status_lines(s: &Studio) -> Vec<String> {
    let mut out: Vec<String> = s
        .session
        .docs()
        .map(|d| {
            format!(
                "{}{} '{}'{}: undo {}, redo {}, {} findings",
                if s.session.active_id() == Some(d.id()) {
                    "* "
                } else {
                    "  "
                },
                d.kind(),
                d.name(),
                if d.is_dirty() { " (changed)" } else { "" },
                d.undo_label().unwrap_or("-"),
                d.redo_label().unwrap_or("-"),
                d.findings().len()
            )
        })
        .collect();
    out.push(match &s.store_dir {
        Some(d) => format!("autosave: {}", shown_path(d)),
        None => "autosave: off (no data directory)".into(),
    });
    out
}

/// A path as shown on screen: the home directory as `~`, so screenshots
/// (published by the art project) never show the user name.
pub fn shown_path(p: &std::path::Path) -> String {
    shown_path_with(
        p,
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .as_deref(),
    )
}

fn shown_path_with(p: &std::path::Path, home: Option<&std::path::Path>) -> String {
    match home.and_then(|h| p.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => p.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_edit::MemoryStore;

    #[test]
    fn paths_on_screen_hide_the_home_directory() {
        use std::path::Path;
        let home = Path::new("/home/ada");
        assert_eq!(
            shown_path_with(
                Path::new("/home/ada/.local/share/ocw/studio/autosave"),
                Some(home)
            ),
            "~/.local/share/ocw/studio/autosave"
        );
        assert_eq!(shown_path_with(home, Some(home)), "~");
        assert_eq!(
            shown_path_with(Path::new("/var/tmp/x"), Some(home)),
            "/var/tmp/x"
        );
        assert_eq!(
            shown_path_with(Path::new("/home/ada/x"), None),
            "/home/ada/x"
        );
    }

    fn studio() -> Studio {
        let mut s = Studio::new(Some(Box::new(MemoryStore::default())), None);
        let id = s.session.open(|id| {
            Document::new(id, "Testland", WorldDoc::new(PackageRef::default()))
                .with_source("/data/testland.ocwpkg")
                .with_validator(Box::new(check_views))
        });
        s.world_doc = Some(id);
        s
    }

    const CAM: ([f64; 3], f64, f64) = ([10.0, 500.0, 20.0], 0.3, -0.4);

    #[test]
    fn views_are_saved_from_the_camera_and_undone() {
        let mut s = studio();
        assert_eq!(
            edit(&mut s, StudioCommand::AddView(None), CAM, 0.0).unwrap(),
            "view 'View 1' saved"
        );
        edit(
            &mut s,
            StudioCommand::AddView(Some("Pass".into())),
            CAM,
            0.5,
        )
        .unwrap();
        let views = &s.world().unwrap().content().views;
        assert_eq!(
            views.len(),
            2,
            "the second add did not merge into the first"
        );
        assert_eq!(views[0].pos, CAM.0);
        assert_eq!(views[0].yaw, CAM.1);
        for (k, n) in ["P", "Pa", "Par"].iter().enumerate() {
            edit(
                &mut s,
                StudioCommand::RenameView(1, (*n).into()),
                CAM,
                1.0 + 0.1 * k as f64,
            )
            .unwrap();
        }
        edit(&mut s, StudioCommand::Seal, CAM, 1.5).unwrap();
        assert_eq!(s.world().unwrap().undo_label(), Some("Rename view"));
        assert_eq!(
            edit(&mut s, StudioCommand::Undo, CAM, 2.0).unwrap(),
            "undone: Rename view"
        );
        assert_eq!(s.world().unwrap().content().views[0].name, "View 1");
        edit(&mut s, StudioCommand::Undo, CAM, 3.0).unwrap();
        edit(&mut s, StudioCommand::Undo, CAM, 4.0).unwrap();
        assert_eq!(
            edit(&mut s, StudioCommand::Undo, CAM, 5.0).unwrap(),
            "nothing to undo"
        );
        assert!(!s.session.any_dirty());
        assert_eq!(
            edit(&mut s, StudioCommand::Redo, CAM, 6.0).unwrap(),
            "redone: Add view"
        );
        assert!(edit(&mut s, StudioCommand::DeleteView(9), CAM, 7.0).is_err());
        assert!(
            edit(&mut s, StudioCommand::GoToView(1), CAM, 7.0).is_err(),
            "a Bevy command"
        );
        let lines = status_lines(&s);
        assert!(
            lines[0].starts_with("* world 'Testland' (changed): undo Add view"),
            "{lines:?}"
        );
    }

    #[test]
    fn an_autosave_is_offered_restored_or_discarded() {
        let mut s = studio();
        edit(
            &mut s,
            StudioCommand::AddView(Some("Lake".into())),
            CAM,
            0.0,
        )
        .unwrap();
        let store = s.store.as_deref_mut().unwrap();
        let (written, _) = s.session.autosave(store, 10.0, 5.0);
        let json = s
            .store
            .as_ref()
            .unwrap()
            .read(&written[0])
            .unwrap()
            .unwrap();
        // A new session of the same package: the autosave is offered.
        let mut t = studio();
        t.restorable = Some(Restorable {
            name: written[0].clone(),
            json,
            summary: "1 views".into(),
        });
        assert_eq!(
            edit(&mut t, StudioCommand::RestoreAutosave, CAM, 0.0).unwrap(),
            "restored: 1 views"
        );
        assert_eq!(t.session.len(), 1, "the fresh document was replaced");
        assert_eq!(t.world().unwrap().content().views[0].name, "Lake");
        edit(&mut t, StudioCommand::Undo, CAM, 1.0).unwrap();
        assert!(
            t.world().unwrap().content().views.is_empty(),
            "with its history"
        );
        assert!(edit(&mut t, StudioCommand::RestoreAutosave, CAM, 2.0).is_err());
        // Discarding removes it from the store.
        let mut u = studio();
        u.store
            .as_deref_mut()
            .unwrap()
            .write("world-x.json", "{}")
            .unwrap();
        u.restorable = Some(Restorable {
            name: "world-x.json".into(),
            json: "{}".into(),
            summary: String::new(),
        });
        edit(&mut u, StudioCommand::DiscardAutosave, CAM, 0.0).unwrap();
        assert_eq!(
            u.store.as_ref().unwrap().list().unwrap(),
            Vec::<String>::new()
        );
    }

    // --- World patches (E5 step 2) ---

    use ocw_world::patch::{Operation, WorldPatch};

    fn tmp_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ocw-studio-edit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A studio with the test package (its file in `dir`) opened.
    fn patch_studio(dir: &std::path::Path) -> Studio {
        let mut s = Studio::new(Some(Box::new(MemoryStore::default())), None);
        let mut pkg = crate::patch::tests::package();
        pkg.file = dir.join("testland.ocwpkg");
        let details = WorldDetails {
            id: "local/testland".into(),
            region: "Testland".into(),
            ..Default::default()
        };
        let file = pkg.file.display().to_string();
        let msg = s.open_package(&file, &file, &details, pkg, 0.0);
        assert_eq!(msg, None, "no patch next to the package yet");
        s
    }

    fn run(s: &mut Studio, c: StudioCommand, t: f64) -> String {
        edit(s, c, CAM, t).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn a_click_selects_a_road_and_its_attributes_are_edited_and_undone() {
        let d = tmp_dir();
        let mut s = patch_studio(&d);
        assert_eq!(
            run(&mut s, StudioCommand::SelectAt([20.0, 3.0], 5.0), 0.0),
            "edge 100 (way 7, edits act on its 2 edge(s)): Hauptstrasse, primary"
        );
        assert_eq!(s.selected_edge(), Some(100));
        // The whole OSM way by default (16H Q18 (a)); the edge is the switch.
        assert_eq!(s.selected_target(), Some(RoadTarget::Way(7)));
        run(&mut s, StudioCommand::WayScope(false), 0.5);
        assert_eq!(s.selected_target(), Some(RoadTarget::Edge(100)));
        run(
            &mut s,
            StudioCommand::SetRoad(RoadChange::Surface(Some("gravel".into()))),
            1.0,
        );
        // Typing a name: one undo step.
        for (k, n) in ["A", "Al", "Alt"].iter().enumerate() {
            run(
                &mut s,
                StudioCommand::SetRoad(RoadChange::Name(Some((*n).into()))),
                2.0 + 0.1 * k as f64,
            );
        }
        run(&mut s, StudioCommand::Seal, 2.5);
        let doc = s.world().unwrap();
        assert_eq!(doc.content().patch.ops.len(), 1);
        assert_eq!(doc.history().undo.len(), 2);
        assert!(doc.is_dirty());
        // The whole way: a second operation, on way 7.
        run(&mut s, StudioCommand::WayScope(true), 3.0);
        assert_eq!(s.selected_target(), Some(RoadTarget::Way(7)));
        run(&mut s, StudioCommand::RemoveRoad, 4.0);
        assert_eq!(
            run(&mut s, StudioCommand::RemoveRoad, 4.5),
            "no change",
            "removed already"
        );
        assert_eq!(s.world().unwrap().content().patch.ops.len(), 2);
        run(&mut s, StudioCommand::Undo, 5.0);
        run(&mut s, StudioCommand::Undo, 5.1);
        run(&mut s, StudioCommand::Undo, 5.2);
        assert!(s.world().unwrap().content().patch.ops.is_empty());
        assert!(!s.world().unwrap().is_dirty());
        // A click beside every road clears the selection.
        assert_eq!(
            run(&mut s, StudioCommand::SelectAt([20.0, 20.0], 5.0), 6.0),
            "no road here"
        );
        assert_eq!(s.selected_edge(), None);
        assert!(edit(&mut s, StudioCommand::RemoveRoad, CAM, 7.0).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn height_points_are_placed_on_the_selected_road_moved_and_removed() {
        let d = tmp_dir();
        let mut s = patch_studio(&d);
        assert!(edit(&mut s, StudioCommand::AddPoint(None), CAM, 0.0).is_err());
        run(&mut s, StudioCommand::SelectEdge(100), 0.0);
        let msg = run(&mut s, StudioCommand::AddPoint(Some([30.0, 4.0])), 1.0);
        assert_eq!(msg, "height point 'point-1' at 400.0 m on edge 100");
        assert_eq!(s.selected_point().as_deref(), Some("point-1"));
        let doc = s.world().unwrap();
        let (_, h) = doc.content().height_point("point-1").unwrap();
        assert_eq!(h.edge, Some(EdgeId(100)));
        assert_eq!(h.radius_m, Some(ocw_world::patch::DEFAULT_RADIUS_M));
        let at = s.point_xz(h).unwrap();
        assert!(
            (at[0] - 30.0).abs() < 0.01 && at[1].abs() < 0.01,
            "on the road: {at:?}"
        );
        // A click on the point selects it again, before the road under it.
        run(&mut s, StudioCommand::ClearSelection, 2.0);
        run(&mut s, StudioCommand::SelectAt([31.0, 1.0], 5.0), 2.0);
        assert_eq!(s.selected_point().as_deref(), Some("point-1"));
        for (k, h) in [401.0, 402.5].iter().enumerate() {
            run(
                &mut s,
                StudioCommand::ChangePoint("point-1".into(), PointChange::Height(*h)),
                3.0 + 0.1 * k as f64,
            );
        }
        run(
            &mut s,
            StudioCommand::ChangePoint("point-1".into(), PointChange::Radius(60.0)),
            3.3,
        );
        run(&mut s, StudioCommand::Seal, 3.4);
        run(
            &mut s,
            StudioCommand::ChangePoint("point-1".into(), PointChange::Raise(-1.5)),
            5.0,
        );
        let doc = s.world().unwrap();
        let (_, h) = doc.content().height_point("point-1").unwrap();
        assert_eq!(
            (h.height_m, h.raise_m, h.radius_m),
            (None, Some(-1.5), Some(60.0))
        );
        // Moved to the crossing path: the point follows the road it is on.
        run(
            &mut s,
            StudioCommand::MovePoint("point-1".into(), [52.0, 30.0]),
            10.0,
        );
        let doc = s.world().unwrap();
        let (_, h) = doc.content().height_point("point-1").unwrap();
        assert_eq!(h.edge, Some(EdgeId(200)));
        let at = s.point_xz(h).unwrap();
        assert!((at[0] - 50.0).abs() < 0.01 && (at[1] - 30.0).abs() < 0.01);
        assert!(
            edit(
                &mut s,
                StudioCommand::MovePoint("point-1".into(), [300.0, 300.0]),
                CAM,
                11.0
            )
            .is_err(),
            "no road there"
        );
        run(&mut s, StudioCommand::RemovePoint("point-1".into()), 12.0);
        assert!(s.world().unwrap().content().patch.ops.is_empty());
        assert_eq!(s.selected_point(), None);
        run(&mut s, StudioCommand::Undo, 13.0);
        run(&mut s, StudioCommand::SelectOp(0), 14.0);
        assert_eq!(s.selected_point().as_deref(), Some("point-1"));
        run(&mut s, StudioCommand::DeleteOp(0), 15.0);
        assert!(s.world().unwrap().content().patch.ops.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_and_open_give_the_identical_patch_and_a_foreign_one_is_refused() {
        let d = tmp_dir();
        let mut s = patch_studio(&d);
        run(&mut s, StudioCommand::SelectEdge(200), 0.0);
        run(
            &mut s,
            StudioCommand::SetRoad(RoadChange::Width(Some(2.5))),
            0.0,
        );
        run(
            &mut s,
            StudioCommand::SetRoad(RoadChange::Oneway(Some(PatchOneway::Backward))),
            0.5,
        );
        run(&mut s, StudioCommand::AddPoint(None), 1.0);
        run(
            &mut s,
            StudioCommand::SetInfo(PatchInfo {
                title: Some("Testland fixes".into()),
                author: Some("Ada Rider".into()),
                description: None,
            }),
            2.0,
        );
        let before = s.world().unwrap().content().patch.clone();
        let saved = run(&mut s, StudioCommand::SavePatch, 3.0);
        let path = d.join("testland.ocwpatch");
        assert!(
            saved.starts_with("saved ") && saved.ends_with("(2 operations)"),
            "{saved}"
        );
        assert!(path.is_file(), "next to the package by default");
        assert!(!s.world().unwrap().is_dirty());
        assert_eq!(s.patch_file.as_deref(), Some(path.as_path()));
        // Opening it again (a new session): the identical patch, clean.
        let mut t = Studio::new(None, None);
        let mut pkg = crate::patch::tests::package();
        pkg.file = d.join("testland.ocwpkg");
        let details = WorldDetails {
            id: "local/testland".into(),
            region: "Testland".into(),
            ..Default::default()
        };
        let file = pkg.file.display().to_string();
        // It asks first (16H Q16 (b)); Load it: the patch as the start.
        let msg = t.open_package(&file, &file, &details, pkg, 0.0).unwrap();
        assert!(
            msg.starts_with("This world has a patch: 2 changes (by Ada Rider"),
            "{msg}"
        );
        assert!(t.world().unwrap().content().patch.ops.is_empty());
        let msg = t.answer_patch(true, 0.0).unwrap();
        assert!(msg.contains("loaded (2 operations)"), "{msg}");
        assert_eq!(t.world().unwrap().content().patch, before);
        assert!(!t.world().unwrap().is_dirty());
        assert!(!t.world().unwrap().can_undo(), "the start, not a step");
        assert_eq!(t.patch_file.as_deref(), Some(path.as_path()));
        // File → Open patch… onto a changed document: replaced, undoable.
        run(&mut t, StudioCommand::DeleteOp(0), 1.0);
        let other = d.join("other.ocwpatch");
        run(
            &mut t,
            StudioCommand::SavePatchAs(other.display().to_string()),
            2.0,
        );
        let opened = run(
            &mut t,
            StudioCommand::OpenPatch(path.display().to_string()),
            3.0,
        );
        assert!(opened.contains("opened (2 operations)"), "{opened}");
        assert_eq!(t.world().unwrap().content().patch, before);
        assert!(!t.world().unwrap().is_dirty());
        assert_eq!(t.world().unwrap().undo_label(), Some("Open patch"));
        // A patch for another package is refused, the document unchanged.
        let mut foreign = WorldPatch::new("europe/elsewhere");
        foreign.ops = before.ops.clone();
        let fp = d.join("foreign.ocwpatch");
        crate::patch::write_patch(&fp, &foreign).unwrap();
        let e = edit(
            &mut t,
            StudioCommand::OpenPatch(fp.display().to_string()),
            CAM,
            4.0,
        )
        .unwrap_err();
        assert!(
            e.contains("`europe/elsewhere`") && e.contains("not loaded"),
            "{e}"
        );
        assert_eq!(t.world().unwrap().content().patch, before);
        // A patch with errors is not saved.
        let id = t.world_doc.unwrap();
        t.session
            .execute::<WorldDoc>(
                id,
                WorldOp::push(Operation::RemoveRoad(Default::default())),
                5.0,
            )
            .unwrap();
        let e = edit(&mut t, StudioCommand::SavePatch, CAM, 6.0).unwrap_err();
        assert!(e.starts_with("not saved: the patch has 1 error"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Opens the test package in `dir` with `choice` (16H Q16).
    fn open_with(
        dir: &std::path::Path,
        choice: crate::patch_prompt::PatchChoice,
    ) -> (Studio, Option<String>) {
        let mut s = Studio::new(None, None);
        s.patch_choice = choice;
        let mut pkg = crate::patch::tests::package();
        pkg.file = dir.join("testland.ocwpkg");
        let details = WorldDetails {
            id: "local/testland".into(),
            region: "Testland".into(),
            ..Default::default()
        };
        let file = pkg.file.display().to_string();
        let msg = s.open_package(&file, &file, &details, pkg, 0.0);
        (s, msg)
    }

    /// A patch with `n` changes for the test package.
    fn test_patch(n: usize, author: &str) -> ocw_world::patch::WorldPatch {
        let mut p = ocw_world::patch::WorldPatch::new("local/testland");
        p.author = Some(author.into());
        for i in 0..n {
            p.ops.push(ocw_world::patch::Operation::RemoveRoad(
                ocw_world::patch::RemoveRoad {
                    way: Some(70 + i as i64),
                    edge: None,
                    note: None,
                },
            ));
        }
        p
    }

    /// Spec 16H Q16 (b): the patch next to the package is asked about –
    /// not loaded until *Load it*; *Open without it* keeps the file (a
    /// later save keeps it as `.bak`); `--patch load|skip` answers before;
    /// the answer is not remembered (the next open asks again).
    #[test]
    fn the_patch_next_to_a_package_is_asked_about_first() {
        use crate::patch_prompt::PatchChoice;
        let d = tmp_dir();
        let file = d.join("testland.ocwpatch");
        crate::patch::write_patch(&file, &test_patch(3, "Ada Rider")).unwrap();
        // Ask (the default): nothing loaded yet.
        let (mut s, msg) = open_with(&d, PatchChoice::Ask);
        let msg = msg.unwrap();
        assert!(
            msg.starts_with("This world has a patch: 3 changes (by Ada Rider, "),
            "{msg}"
        );
        assert!(msg.contains("studio patch load | skip"), "{msg}");
        assert!(s.patch_question.is_some());
        assert!(s.world().unwrap().content().patch.ops.is_empty());
        // A save while it waits does not overwrite the file unasked.
        run(&mut s, StudioCommand::SelectEdge(200), 0.1);
        run(&mut s, StudioCommand::RemoveRoad, 0.2);
        let saved = run(&mut s, StudioCommand::SavePatch, 0.3);
        assert!(saved.contains("kept as"), "{saved}");
        let bak = d.join("testland.ocwpatch.bak");
        assert_eq!(
            crate::patch::read_patch(&bak, "local/testland")
                .unwrap()
                .ops
                .len(),
            3
        );
        std::fs::remove_file(&bak).unwrap();
        crate::patch::write_patch(&file, &test_patch(3, "Ada Rider")).unwrap();
        // Load it, with changes in the document already: one undo step.
        let msg = run(&mut s, StudioCommand::PatchAnswer(true), 1.0);
        assert!(msg.contains("loaded (3 operations)"), "{msg}");
        assert_eq!(s.world().unwrap().content().patch.ops.len(), 3);
        assert!(s.patch_question.is_none());
        run(&mut s, StudioCommand::Undo, 2.0);
        assert_eq!(s.world().unwrap().content().patch.ops.len(), 1);
        assert!(
            edit(&mut s, StudioCommand::PatchAnswer(false), CAM, 3.0).is_err(),
            "nothing waits"
        );
        // Open without it: the document stays empty, the file untouched;
        // the first save keeps the file as `.bak`.
        let (mut s, _) = open_with(&d, PatchChoice::Ask);
        let msg = run(&mut s, StudioCommand::PatchAnswer(false), 0.0);
        assert!(msg.starts_with("opened without the patch"), "{msg}");
        assert!(s.world().unwrap().content().patch.ops.is_empty());
        assert_eq!(
            std::fs::read_to_string(&file)
                .unwrap()
                .matches("[[op]]")
                .count(),
            3
        );
        run(&mut s, StudioCommand::SelectEdge(200), 0.1);
        run(&mut s, StudioCommand::RemoveRoad, 0.2);
        let saved = run(&mut s, StudioCommand::SavePatch, 0.3);
        assert!(saved.contains("testland.ocwpatch.bak"), "{saved}");
        assert_eq!(
            crate::patch::read_patch(&bak, "local/testland")
                .unwrap()
                .ops
                .len(),
            3,
            "the patch that was there"
        );
        assert_eq!(
            crate::patch::read_patch(&file, "local/testland")
                .unwrap()
                .ops
                .len(),
            1,
            "the new one"
        );
        // Loaded later by the console word: one undoable step.
        let msg = run(&mut s, StudioCommand::PatchAnswer(true), 1.0);
        assert!(msg.contains("loaded (1 operations)"), "{msg}");
        // Not remembered: the next open asks again.
        let (s, _) = open_with(&d, PatchChoice::Ask);
        assert!(s.patch_question.is_some());
        // `--patch load`: loaded at once as the start; `--patch skip`: not.
        let (s, msg) = open_with(&d, PatchChoice::Load);
        assert!(msg.unwrap().contains("--patch load"));
        assert!(s.patch_question.is_none());
        assert_eq!(s.world().unwrap().content().patch.ops.len(), 1);
        assert!(!s.world().unwrap().can_undo());
        let (s, msg) = open_with(&d, PatchChoice::Skip);
        assert!(msg.unwrap().contains("not loaded (--patch skip)"));
        assert!(s.world().unwrap().content().patch.ops.is_empty());
        assert!(s.patch_question.is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A stack next to the package (16H Q9 / Q10) is loaded as one patch
    /// in order and saved only with Save as.
    #[test]
    fn a_patch_stack_is_loaded_as_one_and_saved_with_save_as() {
        use crate::patch_prompt::PatchChoice;
        let d = tmp_dir();
        crate::patch::write_patch(&d.join("testland.ocwpatch"), &test_patch(2, "Ada Rider"))
            .unwrap();
        crate::patch::write_patch(
            &d.join("testland.020-bridge.ocwpatch"),
            &test_patch(1, "Bea Climber"),
        )
        .unwrap();
        let (mut s, msg) = open_with(&d, PatchChoice::Ask);
        assert!(msg.unwrap().contains("3 changes in 2 files"));
        run(&mut s, StudioCommand::PatchAnswer(true), 0.0);
        assert_eq!(s.world().unwrap().content().patch.ops.len(), 3);
        assert_eq!(s.patch_file, None);
        let e = edit(&mut s, StudioCommand::SavePatch, CAM, 1.0).unwrap_err();
        assert!(e.contains("Save patch as"), "{e}");
        let one = d.join("merged.ocwpatch");
        run(
            &mut s,
            StudioCommand::SavePatchAs(one.display().to_string()),
            2.0,
        );
        assert_eq!(
            crate::patch::read_patch(&one, "local/testland")
                .unwrap()
                .ops
                .len(),
            3
        );
        // Now saved to its own file: Save works.
        run(&mut s, StudioCommand::SavePatch, 3.0);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Spec 16H Q18 (a): a road edit acts on the whole OSM way by default –
    /// a `way` operation, which the preview applies to every edge of the
    /// way – with the single edge as the switch; a patch with edge edits
    /// (as made before) still opens, applies and is edited on its edge.
    #[test]
    fn road_edits_act_on_the_way_by_default_and_old_edge_edits_still_work() {
        use ocw_world::patch::Operation as Op;
        let d = tmp_dir();
        let mut s = patch_studio(&d);
        run(&mut s, StudioCommand::SelectEdge(100), 0.0);
        assert_eq!(s.selected_target(), Some(RoadTarget::Way(7)));
        run(
            &mut s,
            StudioCommand::SetRoad(RoadChange::Surface(Some("gravel".into()))),
            1.0,
        );
        let ops = s.world().unwrap().content().patch.ops.clone();
        let Op::Road(r) = &ops[0] else {
            panic!("{ops:?}")
        };
        assert_eq!((r.way, r.edge), (Some(7), None), "an OSM way edit");
        assert_eq!(r.surface.as_deref(), Some("gravel"));
        let pkg = s.package.as_ref().unwrap();
        let p = crate::patch::Preview::of(pkg, &s.world().unwrap().content().patch, "t");
        assert!(
            matches!(p.status(0), crate::patch::OpStatus::Ok(ref m) if m.contains("2 edge")),
            "both edges of way 7: {:?}",
            p.status(0)
        );
        // The switch: the single edge – a second operation on edge 100.
        run(&mut s, StudioCommand::WayScope(false), 2.0);
        run(
            &mut s,
            StudioCommand::SetRoad(RoadChange::Width(Some(3.0))),
            3.0,
        );
        run(&mut s, StudioCommand::Seal, 3.1);
        let ops = s.world().unwrap().content().patch.ops.clone();
        assert_eq!(ops.len(), 2);
        let Op::Road(r) = &ops[1] else { panic!() };
        assert_eq!((r.way, r.edge), (None, Some(EdgeId(100))));
        // An older patch with an edge edit: opened, applied, and selecting
        // its operation edits that edge (not the way).
        let mut old = ocw_world::patch::WorldPatch::new("local/testland");
        old.ops.push(Op::Road(ocw_world::patch::RoadEdit {
            edge: Some(EdgeId(300)),
            name: Some("Alter Weg".into()),
            ..Default::default()
        }));
        let f = d.join("old.ocwpatch");
        crate::patch::write_patch(&f, &old).unwrap();
        let mut t = patch_studio(&d);
        run(
            &mut t,
            StudioCommand::OpenPatch(f.display().to_string()),
            0.0,
        );
        let pkg = t.package.as_ref().unwrap();
        let p = crate::patch::Preview::of(pkg, &t.world().unwrap().content().patch, "old");
        assert_eq!(p.status(0).tag(), "ok", "{:?}", p.status(0));
        assert!(t.way_scope);
        // Selected by a click, its way is the target; the panel notes the
        // edge's own edit, which applies over the way's.
        run(&mut t, StudioCommand::SelectEdge(300), 0.5);
        assert_eq!(t.selected_target(), Some(RoadTarget::Way(7)));
        let c = t.world().unwrap().content();
        assert!(crate::ui::way_target_edit_note(c, RoadTarget::Way(7), 300));
        assert!(!crate::ui::way_target_edit_note(
            c,
            RoadTarget::Edge(300),
            300
        ));
        assert!(!crate::ui::way_target_edit_note(c, RoadTarget::Way(7), 100));
        run(&mut t, StudioCommand::SelectOp(0), 1.0);
        assert_eq!(t.selected_target(), Some(RoadTarget::Edge(300)));
        run(
            &mut t,
            StudioCommand::SetRoad(RoadChange::Name(Some("Neuer Weg".into()))),
            2.0,
        );
        let ops = &t.world().unwrap().content().patch.ops;
        assert_eq!(ops.len(), 1, "the same operation");
        let Op::Road(r) = &ops[0] else { panic!() };
        assert_eq!(
            (r.edge, r.name.as_deref()),
            (Some(EdgeId(300)), Some("Neuer Weg"))
        );
        // The console's switch.
        let w = |t: &str| t.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(
            console_patch("road", &w("scope way"), &t).unwrap(),
            Console::Run(StudioCommand::WayScope(true))
        );
        assert_eq!(
            console_patch("patch", &w("load"), &t).unwrap(),
            Console::Run(StudioCommand::PatchAnswer(true))
        );
        assert_eq!(
            console_patch("patch", &w("skip"), &t).unwrap(),
            Console::Run(StudioCommand::PatchAnswer(false))
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_console_words_become_patch_commands() {
        let d = tmp_dir();
        let mut s = patch_studio(&d);
        let w = |t: &str| t.split(' ').map(String::from).collect::<Vec<_>>();
        let c = |s: &Studio, g: &str, t: &str| console_patch(g, &w(t), s);
        assert_eq!(
            c(&s, "select", "20 3").unwrap(),
            Console::Run(StudioCommand::SelectAt([20.0, 3.0], 50.0))
        );
        let Console::Run(StudioCommand::SelectAt(p, _)) = c(&s, "select", "latlon 47 9").unwrap()
        else {
            panic!()
        };
        assert!(p[0].abs() < 1e-9 && p[1].abs() < 1e-9, "the origin");
        assert_eq!(
            c(&s, "select", "op 2").unwrap(),
            Console::Run(StudioCommand::SelectOp(1))
        );
        assert_eq!(
            c(&s, "road", "name Alter Weg").unwrap(),
            Console::Run(StudioCommand::SetRoad(RoadChange::Name(Some(
                "Alter Weg".into()
            ))))
        );
        assert_eq!(
            c(&s, "road", "width -").unwrap(),
            Console::Run(StudioCommand::SetRoad(RoadChange::Width(None)))
        );
        assert_eq!(
            c(&s, "road", "class track").unwrap(),
            Console::Run(StudioCommand::SetRoad(RoadChange::Class(Some(
                PatchClass::Track
            ))))
        );
        assert!(c(&s, "road", "class lava").is_err());
        assert!(c(&s, "road", "oneway up").is_err());
        assert!(c(&s, "point", "raise 3").is_err(), "no point selected");
        run(&mut s, StudioCommand::SelectEdge(100), 0.0);
        run(&mut s, StudioCommand::AddPoint(None), 1.0);
        assert_eq!(
            c(&s, "point", "raise -2.5").unwrap(),
            Console::Run(StudioCommand::ChangePoint(
                "point-1".into(),
                PointChange::Raise(-2.5)
            ))
        );
        let Console::Say(list) = c(&s, "patch", "list").unwrap() else {
            panic!()
        };
        assert!(list.contains("1 operations (changed)") && list.contains("height_point `point-1`"));
        let Console::Run(StudioCommand::Build(b)) = c(&s, "patch", "build").unwrap() else {
            panic!()
        };
        assert_eq!(b.out, d.join("patched").join("testland.ocwpkg"));
        assert!(
            !b.sources.missing().is_empty(),
            "no sources in the test folder"
        );
        let e = start_build(&mut s, b).unwrap_err();
        assert!(e.contains("source data") && e.contains(".osm.pbf"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
