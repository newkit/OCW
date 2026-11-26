//! The World workspace's document (spec §14 Phase 16, 16C): which package
//! is open, the views saved in it – named camera positions to fly back to
//! (studio bookmarks, saved in the world patch file with it, 16H Q13 (b))
//! – and the **world patch** being edited (16C (b), E5 step 2).
//!
//! **The patch is the document's content** (`ocw_world::patch::WorldPatch`,
//! saved as `<package>.ocwpatch`): every edit of it is a [`WorldOp`] with
//! its exact inverse – an operation inserted, removed or replaced, the
//! header changed, or the whole patch replaced (a file opened) – so undo /
//! redo, dirty flags and autosave work for it unchanged. The helpers on
//! [`WorldDoc`] ([`WorldDoc::set_road`], [`WorldDoc::remove_road`],
//! [`WorldDoc::add_height_point`], …) turn what the user does into those
//! operations: one `road` operation per target collects its attribute
//! changes, a height point defined in this patch is moved or removed by
//! changing or deleting its own operation. The validators reuse the patch
//! module's checks ([`check_patch`]: the format rules, the package id) and
//! check the targets against the package's road graph ([`check_targets`]).
//! The saved views are autosaved with the document and travel in the patch
//! file (16H Q13 (b), [`WorldDoc::patch_for_file`]); a patch opened with
//! views offers those the document lacks ([`WorldDoc::views_to_offer`]).

use std::collections::BTreeSet;

use ocw_world::graph::{EdgeId, RoadGraph};
use ocw_world::patch::{
    HeightPoint, Operation, PatchClass, PatchOneway, RemoveRoad, RoadEdit, WorldPatch,
};
use serde::{Deserialize, Serialize};

use crate::document::{DocumentKind, EditError};
use crate::validate::{Finding, Validator};

/// The package a world document is about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackageRef {
    /// The `--world` path or id it was opened with.
    pub spec: String,
    /// Manifest id and version (`local/<name>`, 0 without a manifest).
    pub id: String,
    pub version: u64,
}

/// A saved view: where the camera was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// Unique within the document, never reused (0: not given yet).
    pub id: u32,
    pub name: String,
    /// The eye (home package metres; x, y up, z).
    pub pos: [f64; 3],
    /// Yaw (rad, counter-clockwise from north) and pitch (rad, up).
    pub yaw: f64,
    pub pitch: f64,
}

impl View {
    /// A view at the origin (tests).
    pub fn named(name: &str) -> Self {
        Self {
            id: 0,
            name: name.into(),
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

/// The World workspace's document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawWorldDoc")]
pub struct WorldDoc {
    pub package: PackageRef,
    pub views: Vec<View>,
    /// The next view id.
    pub next_view: u32,
    /// The world patch for the package (its `package` is the package id).
    pub patch: WorldPatch,
}

/// A world document as stored: autosaves of the E1 studio have no patch.
#[derive(Deserialize)]
struct RawWorldDoc {
    package: PackageRef,
    views: Vec<View>,
    next_view: u32,
    #[serde(default)]
    patch: Option<WorldPatch>,
}

impl From<RawWorldDoc> for WorldDoc {
    fn from(r: RawWorldDoc) -> Self {
        let patch = r
            .patch
            .unwrap_or_else(|| WorldPatch::new(r.package.id.clone()));
        Self {
            package: r.package,
            views: r.views,
            next_view: r.next_view,
            patch,
        }
    }
}

impl Default for WorldDoc {
    fn default() -> Self {
        Self::new(PackageRef::default())
    }
}

/// The patch's header (what people read about it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PatchInfo {
    pub title: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
}

/// The World document's commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorldOp {
    /// Inserts a view at `index` (the end if past it); a view with id 0
    /// gets the next free id.
    Add { view: View, index: usize },
    /// Removes the view with this id.
    Remove(u32),
    /// Renames a view (typing merges into one undo step).
    Rename { id: u32, name: String },
    /// Moves a view to another camera position.
    Move {
        id: u32,
        pos: [f64; 3],
        yaw: f64,
        pitch: f64,
    },
    /// Inserts a patch operation at `index` (the end if past it).
    InsertOp { index: usize, op: Operation },
    /// Removes the patch operation at `index`; `label` names the step in
    /// the Edit menu ("Restore road", "Remove height point").
    RemoveOp { index: usize, label: String },
    /// Replaces the patch operation at `index` (the same one changed again
    /// soon after merges: typing a name, dragging a height).
    ReplaceOp { index: usize, op: Operation },
    /// Replaces the patch's title, author and description.
    SetInfo(PatchInfo),
    /// Replaces the whole patch (a patch file opened).
    SetPatch(Box<WorldPatch>),
}

impl WorldOp {
    /// Adds `view` at the end.
    pub fn add(view: View) -> Self {
        Self::Add {
            view,
            index: usize::MAX,
        }
    }

    /// Appends a patch operation.
    pub fn push(op: Operation) -> Self {
        Self::InsertOp {
            index: usize::MAX,
            op,
        }
    }
}

/// A road a patch operation is about: an OSM way (all its edges) or one
/// stable edge (§4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RoadTarget {
    Way(i64),
    Edge(u64),
}

impl RoadTarget {
    /// The target of a road or removal operation's fields.
    pub fn of(way: Option<i64>, edge: Option<EdgeId>) -> Option<Self> {
        match (way, edge) {
            (Some(w), None) => Some(Self::Way(w)),
            (None, Some(e)) => Some(Self::Edge(e.0)),
            _ => None,
        }
    }

    /// The `way` / `edge` fields of an operation on this target.
    pub fn fields(self) -> (Option<i64>, Option<EdgeId>) {
        match self {
            Self::Way(w) => (Some(w), None),
            Self::Edge(e) => (None, Some(EdgeId(e))),
        }
    }

    /// Whether an edge (its id and OSM way) is this target.
    pub fn hits(self, edge: u64, osm_way: i64) -> bool {
        match self {
            Self::Way(w) => w == osm_way,
            Self::Edge(e) => e == edge,
        }
    }

    /// "edge 123", "way 45".
    pub fn describe(self) -> String {
        match self {
            Self::Way(w) => format!("way {w}"),
            Self::Edge(e) => format!("edge {e}"),
        }
    }
}

/// The road target of a patch operation, if it has one.
pub fn op_road_target(op: &Operation) -> Option<RoadTarget> {
    match op {
        Operation::Road(r) => RoadTarget::of(r.way, r.edge),
        Operation::RemoveRoad(r) => RoadTarget::of(r.way, r.edge),
        _ => None,
    }
}

/// The height point id a patch operation is about, if any.
pub fn op_point_id(op: &Operation) -> Option<&str> {
    match op {
        Operation::HeightPoint(h) => Some(&h.id),
        Operation::MoveHeightPoint(m) => Some(&m.id),
        Operation::RemoveHeightPoint(r) => Some(&r.id),
        _ => None,
    }
}

/// One attribute of a road changed in the road panel; `None` takes the
/// attribute out of the patch again (the package's value counts).
#[derive(Debug, Clone, PartialEq)]
pub enum RoadChange {
    /// An OSM `surface` value.
    Surface(Option<String>),
    Width(Option<f64>),
    Class(Option<PatchClass>),
    Oneway(Option<PatchOneway>),
    /// Closed for riding (`bike = "closed"`).
    BikeClosed(bool),
    /// The name; `Some("")` removes the package's name.
    Name(Option<String>),
}

impl RoadChange {
    fn apply_to(&self, r: &mut RoadEdit) {
        match self {
            Self::Surface(v) => r.surface = v.clone(),
            Self::Width(v) => r.width_m = *v,
            Self::Class(v) => r.class = *v,
            Self::Oneway(v) => r.oneway = *v,
            Self::BikeClosed(v) => r.bike = v.then_some(ocw_world::patch::BikeAccess::Closed),
            Self::Name(v) => r.name = v.clone(),
        }
    }
}

/// Whether a road edit changes anything.
fn edits_something(r: &RoadEdit) -> bool {
    r.surface.is_some()
        || r.width_m.is_some()
        || r.class.is_some()
        || r.oneway.is_some()
        || r.bike.is_some()
        || r.name.is_some()
}

impl WorldDoc {
    pub fn new(package: PackageRef) -> Self {
        let patch = WorldPatch::new(package.id.clone());
        Self {
            package,
            views: Vec::new(),
            next_view: 1,
            patch,
        }
    }

    pub fn view(&self, id: u32) -> Option<&View> {
        self.views.iter().find(|v| v.id == id)
    }

    fn index(&self, id: u32) -> Result<usize, EditError> {
        self.views
            .iter()
            .position(|v| v.id == id)
            .ok_or_else(|| EditError::new(format!("no view {id}")))
    }

    /// A name for a new view: "View 1", "View 2", … not taken yet.
    pub fn new_view_name(&self) -> String {
        (1..)
            .map(|n| format!("View {n}"))
            .find(|n| !self.views.iter().any(|v| &v.name == n))
            .expect("some number is free")
    }

    /// The patch as its file holds it: the operations and the saved views
    /// (16H Q13 (b): the views travel with the patch; those of an older
    /// studio's autosave move into the file on its next save).
    pub fn patch_for_file(&self) -> WorldPatch {
        let mut p = self.patch.clone();
        p.views = self
            .views
            .iter()
            .map(|v| ocw_world::patch::PatchView {
                name: v.name.clone(),
                pos: v.pos,
                yaw: v.yaw,
                pitch: v.pitch,
            })
            .collect();
        p
    }

    /// The views of a patch file that this document does not have yet
    /// (the same name at the same place counts as had), to offer them
    /// when the patch is opened (16H Q13 (b)); new ids are given on add.
    pub fn views_to_offer(&self, patch: &WorldPatch) -> Vec<View> {
        patch
            .views
            .iter()
            .filter(|p| {
                !self
                    .views
                    .iter()
                    .any(|v| v.name == p.name && v.pos == p.pos)
            })
            .map(|p| View {
                id: 0,
                name: p.name.clone(),
                pos: p.pos,
                yaw: p.yaw,
                pitch: p.pitch,
            })
            .collect()
    }

    /// The patch's header.
    pub fn info(&self) -> PatchInfo {
        PatchInfo {
            title: self.patch.title.clone(),
            author: self.patch.author.clone(),
            description: self.patch.description.clone(),
        }
    }

    /// The last `road` operation on `target` (index, edit).
    pub fn road_edit(&self, target: RoadTarget) -> Option<(usize, &RoadEdit)> {
        self.patch
            .ops
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, op)| match op {
                Operation::Road(r) if RoadTarget::of(r.way, r.edge) == Some(target) => Some((i, r)),
                _ => None,
            })
    }

    /// The last `remove_road` operation on `target`.
    pub fn road_removal(&self, target: RoadTarget) -> Option<usize> {
        self.patch.ops.iter().rposition(|op| {
            matches!(op, Operation::RemoveRoad(r) if RoadTarget::of(r.way, r.edge) == Some(target))
        })
    }

    /// The command that changes one attribute of `target`: the target's
    /// `road` operation is changed (or appended, or deleted when nothing
    /// is left in it). `None`: nothing would change.
    pub fn set_road(&self, target: RoadTarget, change: &RoadChange) -> Option<WorldOp> {
        match self.road_edit(target) {
            Some((index, old)) => {
                let mut r = old.clone();
                change.apply_to(&mut r);
                if r == *old {
                    None
                } else if edits_something(&r) {
                    Some(WorldOp::ReplaceOp {
                        index,
                        op: Operation::Road(r),
                    })
                } else {
                    Some(WorldOp::RemoveOp {
                        index,
                        label: "Reset road".into(),
                    })
                }
            }
            None => {
                let (way, edge) = target.fields();
                let mut r = RoadEdit {
                    way,
                    edge,
                    ..Default::default()
                };
                change.apply_to(&mut r);
                edits_something(&r).then(|| WorldOp::push(Operation::Road(r)))
            }
        }
    }

    /// The command that takes `target` out of the ridable graph (`None`:
    /// it already is).
    pub fn remove_road(&self, target: RoadTarget) -> Option<WorldOp> {
        if self.road_removal(target).is_some() {
            return None;
        }
        let (way, edge) = target.fields();
        Some(WorldOp::push(Operation::RemoveRoad(RemoveRoad {
            way,
            edge,
            note: None,
        })))
    }

    /// The command that deletes the removal of `target` (`None`: not
    /// removed by this patch).
    pub fn restore_road(&self, target: RoadTarget) -> Option<WorldOp> {
        self.road_removal(target).map(|index| WorldOp::RemoveOp {
            index,
            label: "Restore road".into(),
        })
    }

    /// The `height_point` operation defining `id` in this patch.
    pub fn height_point(&self, id: &str) -> Option<(usize, &HeightPoint)> {
        self.patch
            .ops
            .iter()
            .enumerate()
            .find_map(|(i, op)| match op {
                Operation::HeightPoint(h) if h.id == id => Some((i, h)),
                _ => None,
            })
    }

    /// The height points this patch defines.
    pub fn height_points(&self) -> impl Iterator<Item = (usize, &HeightPoint)> {
        self.patch
            .ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| match op {
                Operation::HeightPoint(h) => Some((i, h)),
                _ => None,
            })
    }

    /// An id for a new height point: "point-1", "point-2", … not used by
    /// any operation of the patch yet.
    pub fn new_point_id(&self) -> String {
        let used: BTreeSet<&str> = self.patch.ops.iter().filter_map(op_point_id).collect();
        (1..)
            .map(|n| format!("point-{n}"))
            .find(|n| !used.contains(n.as_str()))
            .expect("some number is free")
    }

    /// The command that adds a height point (appended).
    pub fn add_height_point(&self, point: HeightPoint) -> WorldOp {
        WorldOp::push(Operation::HeightPoint(point))
    }

    /// The command that changes the height point `id` defined in this
    /// patch (`None`: no such point, or no change).
    pub fn change_height_point(
        &self,
        id: &str,
        change: impl FnOnce(&mut HeightPoint),
    ) -> Option<WorldOp> {
        let (index, old) = self.height_point(id)?;
        let mut h = old.clone();
        change(&mut h);
        (h != *old).then_some(WorldOp::ReplaceOp {
            index,
            op: Operation::HeightPoint(h),
        })
    }

    /// The command that removes the height point `id` defined in this
    /// patch (its operation is deleted).
    pub fn remove_height_point(&self, id: &str) -> Option<WorldOp> {
        let (index, _) = self.height_point(id)?;
        Some(WorldOp::RemoveOp {
            index,
            label: "Remove height point".into(),
        })
    }

    /// The roads the patch changes (also those closed for bikes: they stay
    /// rideable, with warning signs, 16H Q6) and those it takes out of the
    /// graph (removed), for the previews.
    pub fn touched_roads(&self) -> (BTreeSet<RoadTarget>, BTreeSet<RoadTarget>) {
        let mut changed = BTreeSet::new();
        let mut removed = BTreeSet::new();
        for op in &self.patch.ops {
            match op {
                Operation::Road(r) => {
                    if let Some(t) = RoadTarget::of(r.way, r.edge) {
                        changed.insert(t);
                    }
                }
                Operation::RemoveRoad(r) => {
                    if let Some(t) = RoadTarget::of(r.way, r.edge) {
                        removed.insert(t);
                    }
                }
                _ => {}
            }
        }
        (changed, removed)
    }

    fn op_index(&self, index: usize) -> Result<usize, EditError> {
        (index < self.patch.ops.len())
            .then_some(index)
            .ok_or_else(|| EditError::new(format!("no patch operation {}", index + 1)))
    }
}

/// What can be selected in the World document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorldItem {
    View(u32),
    /// A road edge of the package (stable edge id).
    Edge(u64),
    /// A height point of the patch (its id).
    HeightPoint(String),
}

/// The Edit menu's name of a patch operation added.
fn insert_label(op: &Operation) -> &'static str {
    match op {
        Operation::Road(_) => "Edit road",
        Operation::RemoveRoad(_) => "Remove road",
        Operation::HeightPoint(_) => "Add height point",
        Operation::MoveHeightPoint(_) => "Move height point",
        Operation::RemoveHeightPoint(_) => "Remove height point",
    }
}

impl DocumentKind for WorldDoc {
    type Op = WorldOp;
    type Item = WorldItem;
    const KIND: &'static str = "world";

    fn apply(&mut self, op: &WorldOp) -> Result<WorldOp, EditError> {
        Ok(match op {
            WorldOp::Add { view, index } => {
                let mut view = view.clone();
                if view.id == 0 {
                    view.id = self.next_view.max(1);
                }
                if self.view(view.id).is_some() {
                    return Err(EditError::new(format!("view {} exists", view.id)));
                }
                self.next_view = self.next_view.max(view.id + 1);
                let id = view.id;
                let at = (*index).min(self.views.len());
                self.views.insert(at, view);
                WorldOp::Remove(id)
            }
            WorldOp::Remove(id) => {
                let i = self.index(*id)?;
                let view = self.views.remove(i);
                WorldOp::Add { view, index: i }
            }
            WorldOp::Rename { id, name } => {
                let i = self.index(*id)?;
                let old = std::mem::replace(&mut self.views[i].name, name.clone());
                WorldOp::Rename { id: *id, name: old }
            }
            WorldOp::Move {
                id,
                pos,
                yaw,
                pitch,
            } => {
                let i = self.index(*id)?;
                let v = &mut self.views[i];
                let undo = WorldOp::Move {
                    id: *id,
                    pos: v.pos,
                    yaw: v.yaw,
                    pitch: v.pitch,
                };
                (v.pos, v.yaw, v.pitch) = (*pos, *yaw, *pitch);
                undo
            }
            WorldOp::InsertOp { index, op } => {
                let at = (*index).min(self.patch.ops.len());
                self.patch.ops.insert(at, op.clone());
                WorldOp::RemoveOp {
                    index: at,
                    label: "Delete patch operation".into(),
                }
            }
            WorldOp::RemoveOp { index, .. } => {
                let i = self.op_index(*index)?;
                let op = self.patch.ops.remove(i);
                WorldOp::InsertOp { index: i, op }
            }
            WorldOp::ReplaceOp { index, op } => {
                let i = self.op_index(*index)?;
                let old = std::mem::replace(&mut self.patch.ops[i], op.clone());
                WorldOp::ReplaceOp { index: i, op: old }
            }
            WorldOp::SetInfo(info) => {
                let old = self.info();
                self.patch.title = info.title.clone();
                self.patch.author = info.author.clone();
                self.patch.description = info.description.clone();
                WorldOp::SetInfo(old)
            }
            WorldOp::SetPatch(p) => {
                let old = std::mem::replace(&mut self.patch, (**p).clone());
                WorldOp::SetPatch(Box::new(old))
            }
        })
    }

    fn label(op: &WorldOp) -> String {
        match op {
            WorldOp::Add { .. } => "Add view".into(),
            WorldOp::Remove(_) => "Delete view".into(),
            WorldOp::Rename { .. } => "Rename view".into(),
            WorldOp::Move { .. } => "Move view".into(),
            WorldOp::InsertOp { op, .. } => insert_label(op).into(),
            WorldOp::RemoveOp { label, .. } => label.clone(),
            WorldOp::ReplaceOp { op, .. } => match op {
                Operation::Road(_) => "Edit road".into(),
                Operation::HeightPoint(_) => "Change height point".into(),
                other => format!("Change {}", other.kind()),
            },
            WorldOp::SetInfo(_) => "Edit patch info".into(),
            WorldOp::SetPatch(_) => "Open patch".into(),
        }
    }

    fn merges(prev: &WorldOp, next: &WorldOp) -> bool {
        match (prev, next) {
            (WorldOp::Rename { id: a, .. }, WorldOp::Rename { id: b, .. }) => a == b,
            (WorldOp::Move { id: a, .. }, WorldOp::Move { id: b, .. }) => a == b,
            (WorldOp::ReplaceOp { index: a, op: x }, WorldOp::ReplaceOp { index: b, op: y }) => {
                a == b && x.kind() == y.kind()
            }
            (WorldOp::SetInfo(_), WorldOp::SetInfo(_)) => true,
            _ => false,
        }
    }

    fn contains(&self, item: &WorldItem) -> bool {
        match item {
            WorldItem::View(id) => self.view(*id).is_some(),
            WorldItem::Edge(_) => true,
            WorldItem::HeightPoint(id) => self.height_point(id).is_some(),
        }
    }
}

/// The World document's validator: views have names, and different ones.
pub fn check_views(doc: &WorldDoc) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, v) in doc.views.iter().enumerate() {
        let name = v.name.trim();
        if name.is_empty() {
            out.push(Finding::warning("a view without a name").about(format!("view {}", v.id)));
        } else if doc.views[..i].iter().any(|o| o.name.trim() == name) {
            out.push(Finding::warning(format!("two views are called '{name}'")).about(name));
        }
        if v.pos.iter().any(|c| !c.is_finite()) {
            out.push(Finding::error("a view without a valid position").about(name));
        }
    }
    out
}

/// The finding target of patch operation `i` ("op 3", one-based as in
/// the patch check's report).
pub fn op_finding_target(i: usize) -> String {
    format!("op {}", i + 1)
}

/// The patch's format checks (those of `ocw_world::patch`, every problem
/// at once) and its package id: errors – the patch cannot be saved or
/// built like this.
pub fn check_patch(doc: &WorldDoc) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(m) = doc.patch.header_problem() {
        out.push(Finding::error(m).about("patch"));
    } else if !doc.package.id.is_empty() && doc.patch.package != doc.package.id {
        out.push(
            Finding::error(format!(
                "the patch is for package `{}`, not `{}`",
                doc.patch.package, doc.package.id
            ))
            .about("patch"),
        );
    }
    for (i, m) in doc.patch.op_problems() {
        out.push(
            Finding::error(format!(
                "operation {} ({}): {m}",
                i + 1,
                doc.patch.ops[i].kind()
            ))
            .about(op_finding_target(i)),
        );
    }
    out
}

/// What patch targets the package has: its road graph's edge ids and OSM
/// ways.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PatchTargets {
    pub edges: BTreeSet<u64>,
    pub ways: BTreeSet<i64>,
}

impl PatchTargets {
    pub fn of(graph: &RoadGraph) -> Self {
        Self {
            edges: graph.edges.iter().map(|e| e.id.0).collect(),
            ways: graph.edges.iter().map(|e| e.osm_way).collect(),
        }
    }

    pub fn has(&self, t: RoadTarget) -> bool {
        match t {
            RoadTarget::Way(w) => self.ways.contains(&w),
            RoadTarget::Edge(e) => self.edges.contains(&e),
        }
    }
}

/// Why patch operation `i` finds no target (`None`: it does, as far as
/// the targets and the operations before it tell; height points are
/// placed on the roads by the preview).
pub fn missing_target(patch: &WorldPatch, i: usize, targets: &PatchTargets) -> Option<String> {
    let op = patch.ops.get(i)?;
    if let Some(t) = op_road_target(op) {
        return (!targets.has(t)).then(|| {
            format!(
                "{} is not in the package's road graph (missing: not applied)",
                t.describe()
            )
        });
    }
    if let Operation::MoveHeightPoint(_) | Operation::RemoveHeightPoint(_) = op {
        let id = op_point_id(op)?;
        let mut defined = BTreeSet::new();
        for o in &patch.ops[..i] {
            match o {
                Operation::HeightPoint(h) => {
                    defined.insert(h.id.as_str());
                }
                Operation::RemoveHeightPoint(r) => {
                    defined.remove(r.id.as_str());
                }
                _ => {}
            }
        }
        return (!defined.contains(id))
            .then(|| format!("no height point `{id}` before this (missing: not applied)"));
    }
    None
}

/// A validator that checks the patch's road targets against the package
/// (an unknown edge or way is reported – `world-builder` would report it
/// MISSING and not apply it), and moves / removals of height points not
/// defined before them: warnings, the patch can be saved.
pub fn check_targets(targets: PatchTargets) -> Validator<WorldDoc> {
    Box::new(move |doc: &WorldDoc| {
        (0..doc.patch.ops.len())
            .filter_map(|i| {
                missing_target(&doc.patch, i, &targets).map(|m| {
                    Finding::warning(format!(
                        "operation {} ({}): {m}",
                        i + 1,
                        doc.patch.ops[i].describe()
                    ))
                    .about(op_finding_target(i))
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocId, Document};

    fn doc() -> Document<WorldDoc> {
        Document::new(
            DocId(1),
            "Testland",
            WorldDoc::new(PackageRef {
                spec: "data/testland.ocwpkg".into(),
                id: "local/testland".into(),
                version: 0,
            }),
        )
        .with_validator(Box::new(check_views))
    }

    #[test]
    fn views_are_added_renamed_moved_and_removed_with_undo() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("Bridge")), 0.0).unwrap();
        d.execute(WorldOp::add(View::named("Pass")), 5.0).unwrap();
        let ids: Vec<u32> = d.content().views.iter().map(|v| v.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(d.content().new_view_name(), "View 1");
        d.execute(
            WorldOp::Move {
                id: 2,
                pos: [1.0, 2.0, 3.0],
                yaw: 0.5,
                pitch: -0.1,
            },
            10.0,
        )
        .unwrap();
        d.execute(WorldOp::Remove(1), 15.0).unwrap();
        assert_eq!(d.content().views.len(), 1);
        d.undo(16.0).unwrap();
        assert_eq!(d.content().views[0].name, "Bridge", "back in its place");
        d.undo(17.0).unwrap();
        assert_eq!(d.content().view(2).unwrap().pos, [0.0; 3]);
        // A removed view's id is not given again.
        d.undo(18.0).unwrap();
        d.execute(WorldOp::add(View::named("Lake")), 30.0).unwrap();
        assert_eq!(d.content().view(3).unwrap().name, "Lake");
        assert!(d.execute(WorldOp::Remove(9), 31.0).is_err());
        let mut taken = View::named("x");
        taken.id = 3;
        assert!(d.execute(WorldOp::add(taken), 32.0).is_err());
    }

    #[test]
    fn typing_a_name_is_one_undo_step() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("V")), 0.0).unwrap();
        d.seal();
        for (k, name) in ["S", "St", "Sta", "Star", "Start"].iter().enumerate() {
            d.execute(
                WorldOp::Rename {
                    id: 1,
                    name: (*name).into(),
                },
                1.0 + 0.2 * k as f64,
            )
            .unwrap();
        }
        assert_eq!(d.history().undo.len(), 2);
        assert_eq!(d.undo_label(), Some("Rename view"));
        d.undo(3.0).unwrap();
        assert_eq!(d.content().views[0].name, "V");
    }

    #[test]
    fn the_views_go_into_the_patch_file_and_are_offered_back() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("Bridge")), 0.0).unwrap();
        d.execute(
            WorldOp::add(View {
                pos: [5.0, 600.0, -7.0],
                yaw: 1.0,
                ..View::named("Pass")
            }),
            5.0,
        )
        .unwrap();
        let file = d.content().patch_for_file();
        assert_eq!(file.views.len(), 2);
        assert_eq!(file.views[1].name, "Pass");
        assert_eq!(file.views[1].pos, [5.0, 600.0, -7.0]);
        assert!(
            d.content().patch.views.is_empty(),
            "the document keeps them as views"
        );
        // Read back: the file's views round-trip; a document that has them
        // is offered none, a fresh one both.
        let back = WorldPatch::parse(&file.to_toml()).unwrap();
        assert_eq!(back.views, file.views);
        assert!(d.content().views_to_offer(&back).is_empty());
        let fresh = WorldDoc::new(d.content().package.clone());
        let offered = fresh.views_to_offer(&back);
        assert_eq!(offered.len(), 2);
        assert!(offered.iter().all(|v| v.id == 0));
        // A view of the same name elsewhere is offered (moved since).
        let mut moved = back.clone();
        moved.views[0].pos = [1.0, 1.0, 1.0];
        assert_eq!(d.content().views_to_offer(&moved).len(), 1);
    }

    #[test]
    fn the_validator_reports_unnamed_and_doubled_views() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("A")), 0.0).unwrap();
        assert!(d.findings().is_empty());
        d.execute(WorldOp::add(View::named("A")), 5.0).unwrap();
        assert_eq!(d.findings().len(), 1);
        assert!(d.findings()[0].message.contains("'A'"));
        d.execute(
            WorldOp::Rename {
                id: 2,
                name: " ".into(),
            },
            10.0,
        )
        .unwrap();
        assert!(d.findings()[0].message.contains("without a name"));
        let mut bad = View::named("B");
        bad.pos[0] = f64::NAN;
        d.execute(WorldOp::add(bad), 15.0).unwrap();
        assert_eq!(
            crate::validate::worst(d.findings()),
            Some(crate::Severity::Error)
        );
        d.undo(16.0).unwrap();
        d.undo(17.0).unwrap();
        d.undo(18.0).unwrap();
        assert!(d.findings().is_empty());
    }

    #[test]
    fn a_selected_view_is_dropped_when_it_goes() {
        let mut d = doc();
        d.execute(WorldOp::add(View::named("A")), 0.0).unwrap();
        d.selection.select_only(WorldItem::View(1));
        d.execute(WorldOp::Remove(1), 5.0).unwrap();
        assert!(d.selection.is_empty());
    }

    // --- The world patch (E5 step 2) ---

    use ocw_world::graph::{Edge, Oneway, PROFILE_STEP_M, RoadClass, VertexId};
    use ocw_world::patch::{BikeAccess, MoveHeightPoint, RemoveHeightPoint};

    fn patch_doc() -> Document<WorldDoc> {
        doc()
            .with_validator(Box::new(check_patch))
            .with_validator(check_targets(PatchTargets {
                edges: [10, 11, 12].into(),
                ways: [1, 2].into(),
            }))
    }

    fn point(id: &str) -> HeightPoint {
        HeightPoint {
            id: id.into(),
            at: Some([47.001, 9.501]),
            edge: Some(EdgeId(10)),
            raise_m: Some(2.0),
            radius_m: Some(80.0),
            ..Default::default()
        }
    }

    const E10: RoadTarget = RoadTarget::Edge(10);

    #[test]
    fn a_new_document_has_an_empty_patch_for_its_package() {
        let d = patch_doc();
        assert_eq!(d.content().patch.package, "local/testland");
        assert!(d.content().patch.ops.is_empty());
        assert!(d.findings().is_empty());
    }

    #[test]
    fn road_attributes_collect_in_one_operation_per_target() {
        let mut d = patch_doc();
        let set = |d: &mut Document<WorldDoc>, c: RoadChange, t: f64| {
            let op = d.content().set_road(E10, &c).expect("a change");
            d.execute(op, t).unwrap();
            d.seal();
        };
        set(&mut d, RoadChange::Surface(Some("gravel".into())), 0.0);
        assert_eq!(d.undo_label(), Some("Edit road"));
        set(&mut d, RoadChange::Width(Some(4.5)), 1.0);
        set(&mut d, RoadChange::Class(Some(PatchClass::Track)), 2.0);
        set(&mut d, RoadChange::Oneway(Some(PatchOneway::Forward)), 3.0);
        set(&mut d, RoadChange::Name(Some("Alter Weg".into())), 4.0);
        set(&mut d, RoadChange::BikeClosed(true), 5.0);
        let ops = &d.content().patch.ops;
        assert_eq!(ops.len(), 1, "one road operation for the edge");
        let Operation::Road(r) = &ops[0] else {
            panic!("{ops:?}")
        };
        assert_eq!(r.edge, Some(EdgeId(10)));
        assert_eq!(r.surface.as_deref(), Some("gravel"));
        assert_eq!(r.width_m, Some(4.5));
        assert_eq!(r.bike, Some(BikeAccess::Closed));
        assert_eq!(
            d.content().set_road(E10, &RoadChange::Width(Some(4.5))),
            None,
            "no change, no command"
        );
        // Another target gets its own operation; a way target too.
        let op = d
            .content()
            .set_road(RoadTarget::Way(2), &RoadChange::Name(Some(String::new())))
            .unwrap();
        d.execute(op, 6.0).unwrap();
        assert_eq!(d.content().patch.ops.len(), 2);
        let (changed, removed) = d.content().touched_roads();
        assert_eq!(changed, [RoadTarget::Way(2), E10].into(), "closed for bikes stays");
        assert!(removed.is_empty());
        // Taking every attribute out again deletes the operation.
        for c in [
            RoadChange::Surface(None),
            RoadChange::Width(None),
            RoadChange::Class(None),
            RoadChange::Oneway(None),
            RoadChange::Name(None),
        ] {
            let op = d.content().set_road(E10, &c).unwrap();
            d.execute(op, 10.0).unwrap();
            d.seal();
        }
        let op = d
            .content()
            .set_road(E10, &RoadChange::BikeClosed(false))
            .unwrap();
        assert!(matches!(op, WorldOp::RemoveOp { .. }));
        d.execute(op, 11.0).unwrap();
        assert_eq!(d.undo_label(), Some("Reset road"));
        assert_eq!(d.content().patch.ops.len(), 1);
        assert!(d.content().road_edit(E10).is_none());
        while d.undo(20.0).unwrap() {}
        assert!(d.content().patch.ops.is_empty());
        assert!(!d.is_dirty());
    }

    #[test]
    fn roads_are_removed_and_restored() {
        let mut d = patch_doc();
        let op = d.content().remove_road(E10).unwrap();
        d.execute(op, 0.0).unwrap();
        assert_eq!(d.undo_label(), Some("Remove road"));
        assert!(d.content().remove_road(E10).is_none(), "removed already");
        assert!(d.content().road_removal(E10).is_some());
        let op = d.content().restore_road(E10).unwrap();
        d.execute(op, 5.0).unwrap();
        assert_eq!(d.undo_label(), Some("Restore road"));
        assert!(d.content().patch.ops.is_empty());
        assert!(d.content().restore_road(E10).is_none());
        d.undo(6.0).unwrap();
        assert_eq!(d.content().touched_roads().1, [E10].into());
    }

    #[test]
    fn height_points_are_added_changed_moved_and_removed() {
        let mut d = patch_doc();
        assert_eq!(d.content().new_point_id(), "point-1");
        let op = d.content().add_height_point(point("point-1"));
        d.execute(op, 0.0).unwrap();
        assert_eq!(d.undo_label(), Some("Add height point"));
        assert_eq!(d.content().new_point_id(), "point-2");
        d.selection
            .select_only(WorldItem::HeightPoint("point-1".into()));
        // Dragging the height: one undo step.
        for (k, h) in [1.0, 1.5, 2.5].iter().enumerate() {
            let op = d
                .content()
                .change_height_point("point-1", |p| {
                    p.raise_m = None;
                    p.height_m = Some(400.0 + h);
                })
                .unwrap();
            d.execute(op, 5.0 + 0.2 * k as f64).unwrap();
        }
        assert_eq!(d.history().undo.len(), 2);
        assert_eq!(d.undo_label(), Some("Change height point"));
        assert_eq!(
            d.content().height_point("point-1").unwrap().1.height_m,
            Some(402.5)
        );
        d.seal();
        let op = d
            .content()
            .change_height_point("point-1", |p| p.at = Some([47.002, 9.502]))
            .unwrap();
        d.execute(op, 10.0).unwrap();
        assert!(
            d.content()
                .change_height_point("point-1", |p| p.at = Some([47.002, 9.502]))
                .is_none()
        );
        assert!(d.content().change_height_point("nope", |_| {}).is_none());
        let op = d.content().remove_height_point("point-1").unwrap();
        d.execute(op, 20.0).unwrap();
        assert!(d.selection.is_empty(), "the removed point is deselected");
        d.undo(21.0).unwrap();
        assert_eq!(
            d.content().height_point("point-1").unwrap().1.at,
            Some([47.002, 9.502])
        );
        assert_eq!(d.content().height_points().count(), 1);
    }

    /// Every patch operation's inverse is exact: applying an operation and
    /// then its inverse gives the identical document, and the inverse of
    /// the inverse is the operation again (spec 16G).
    #[test]
    fn every_patch_operation_has_an_exact_inverse() {
        let mut base = WorldDoc::new(PackageRef {
            id: "europe/testland".into(),
            ..Default::default()
        });
        base.patch.ops = vec![
            Operation::Road(RoadEdit {
                way: Some(1),
                surface: Some("sett".into()),
                ..Default::default()
            }),
            Operation::HeightPoint(point("a")),
        ];
        let mut other = WorldPatch::new("europe/testland");
        other.title = Some("Testland fixes".into());
        other.ops = vec![Operation::RemoveRoad(RemoveRoad {
            way: None,
            edge: Some(EdgeId(12)),
            note: Some("closed on site".into()),
        })];
        let ops = vec![
            WorldOp::push(Operation::RemoveRoad(RemoveRoad {
                way: None,
                edge: Some(EdgeId(11)),
                note: None,
            })),
            WorldOp::InsertOp {
                index: 0,
                op: Operation::MoveHeightPoint(MoveHeightPoint {
                    id: "a".into(),
                    raise_m: Some(-1.0),
                    ..Default::default()
                }),
            },
            WorldOp::InsertOp {
                index: 1,
                op: Operation::RemoveHeightPoint(RemoveHeightPoint {
                    id: "a".into(),
                    note: None,
                }),
            },
            WorldOp::RemoveOp {
                index: 1,
                label: "x".into(),
            },
            WorldOp::ReplaceOp {
                index: 0,
                op: Operation::Road(RoadEdit {
                    edge: Some(EdgeId(10)),
                    oneway: Some(PatchOneway::Backward),
                    ..Default::default()
                }),
            },
            WorldOp::SetInfo(PatchInfo {
                title: Some("T".into()),
                author: Some("Ada Rider".into()),
                description: None,
            }),
            WorldOp::SetPatch(Box::new(other)),
        ];
        for op in ops {
            let mut d = base.clone();
            let inverse = d.apply(&op).unwrap();
            assert_ne!(d, base, "{op:?} changes the document");
            let again = d.apply(&inverse).unwrap();
            assert_eq!(d, base, "{op:?} undone exactly");
            let mut e = base.clone();
            e.apply(&op).unwrap();
            let mut f = base.clone();
            f.apply(&again).unwrap();
            assert_eq!(e, f, "{op:?}: the inverse's inverse does the same");
        }
        let mut d = base.clone();
        for bad in [
            WorldOp::RemoveOp {
                index: 2,
                label: String::new(),
            },
            WorldOp::ReplaceOp {
                index: 9,
                op: Operation::HeightPoint(point("b")),
            },
        ] {
            assert!(d.apply(&bad).is_err());
            assert_eq!(d, base, "unchanged on an error");
        }
    }

    /// Random sequences of patch edits, undos and redos undo to the
    /// empty patch and redo to the same end (spec 16G).
    #[test]
    fn random_patch_edits_undo_to_the_identical_document() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for round in 0..100 {
            let mut d = patch_doc();
            let start = d.content().clone();
            let mut now = 0.0;
            for _ in 0..60 {
                now += 0.7;
                let t = [
                    RoadTarget::Edge(10),
                    RoadTarget::Edge(11),
                    RoadTarget::Way(2),
                ][next(3) as usize];
                let c = d.content().clone();
                let op = match next(9) {
                    0 => c.set_road(t, &RoadChange::Surface(Some("gravel".into()))),
                    1 => c.set_road(t, &RoadChange::Width(Some(1.0 + next(29) as f64))),
                    2 => c.set_road(t, &RoadChange::Name(None)),
                    3 => c.remove_road(t),
                    4 => c.restore_road(t),
                    5 => Some(c.add_height_point(point(&c.new_point_id()))),
                    6 => c
                        .height_points()
                        .next()
                        .map(|(_, h)| h.id.clone())
                        .and_then(|id| {
                            c.change_height_point(&id, |p| {
                                p.radius_m = Some(10.0 + next(100) as f64)
                            })
                        }),
                    7 => {
                        if d.undo(now).unwrap() {
                            continue;
                        }
                        None
                    }
                    _ => {
                        if d.redo(now).unwrap() {
                            continue;
                        }
                        None
                    }
                };
                if let Some(op) = op {
                    d.execute(op, now).unwrap();
                }
                assert!(
                    d.findings()
                        .iter()
                        .all(|f| f.severity != crate::Severity::Error),
                    "round {round}: {:?}",
                    d.findings()
                );
            }
            while d.redo(now).unwrap() {}
            let end = d.content().clone();
            while d.undo(now).unwrap() {}
            assert_eq!(d.content(), &start, "round {round}");
            while d.redo(now).unwrap() {}
            assert_eq!(d.content(), &end, "round {round}");
        }
    }

    #[test]
    fn the_validators_report_broken_operations_unknown_targets_and_other_packages() {
        let mut d = patch_doc();
        d.execute(
            WorldOp::push(Operation::Road(RoadEdit {
                edge: Some(EdgeId(10)),
                width_m: Some(40.0),
                ..Default::default()
            })),
            0.0,
        )
        .unwrap();
        assert_eq!(d.findings().len(), 1);
        assert_eq!(d.findings()[0].severity, crate::Severity::Error);
        assert!(
            d.findings()[0].message.contains("1 – 30 m"),
            "{:?}",
            d.findings()
        );
        assert_eq!(d.findings()[0].target.as_deref(), Some("op 1"));
        d.undo(1.0).unwrap();
        assert!(d.findings().is_empty());
        let op = d.content().remove_road(RoadTarget::Edge(99)).unwrap();
        d.execute(op, 2.0).unwrap();
        d.execute(
            WorldOp::push(Operation::RemoveHeightPoint(RemoveHeightPoint {
                id: "ghost".into(),
                note: None,
            })),
            3.0,
        )
        .unwrap();
        let f = d.findings();
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(f[0].message.contains("edge 99 is not in the package"));
        assert!(f[1].message.contains("no height point `ghost`"));
        assert!(f.iter().all(|f| f.severity == crate::Severity::Warning));
        let mut foreign = WorldPatch::new("europe/elsewhere");
        foreign.ops.clear();
        d.execute(WorldOp::SetPatch(Box::new(foreign)), 4.0)
            .unwrap();
        assert!(d.findings()[0].message.contains("`europe/elsewhere`"));
    }

    #[test]
    fn the_targets_come_from_the_road_graph() {
        let e = |id: u64, way: i64| {
            let mut e = Edge {
                id: EdgeId(id),
                from: VertexId(0),
                to: VertexId(1),
                points: vec![[0.0, 0.0], [10.0, 0.0]],
                class: RoadClass::Residential,
                name: None,
                width_m: 5.0,
                bridge: false,
                tunnel: false,
                dead_end_m: None,
                boundary: false,
                oneway: Oneway::No,
                osm_way: way,
                profile: Vec::new(),
            };
            e.profile = vec![0.0; (10.0 / PROFILE_STEP_M) as usize + 1];
            e
        };
        let g = RoadGraph {
            vertices: vec![[0.0, 0.0], [10.0, 0.0]],
            edges: vec![e(5, 50), e(6, 50)],
            boundary_vertices: Default::default(),
        };
        let t = PatchTargets::of(&g);
        assert!(t.has(RoadTarget::Edge(6)) && t.has(RoadTarget::Way(50)));
        assert!(!t.has(RoadTarget::Edge(50)));
        assert!(RoadTarget::Way(50).hits(5, 50) && !RoadTarget::Edge(6).hits(5, 50));
    }

    #[test]
    fn the_patch_survives_autosave_json_and_old_autosaves_get_one() {
        let mut d = patch_doc();
        let op = d.content().add_height_point(point("p"));
        d.execute(op, 0.0).unwrap();
        let op = d
            .content()
            .set_road(RoadTarget::Edge(u64::MAX), &RoadChange::Width(Some(3.0)))
            .unwrap();
        d.execute(op, 5.0).unwrap();
        let json = d.autosave_record().to_json().unwrap();
        let r: crate::AutosaveRecord<WorldDoc, WorldOp> =
            crate::AutosaveRecord::from_json(&json).unwrap();
        assert_eq!(&r.content, d.content(), "edge ids beyond i64 too");
        // An autosave of the E1 studio (views only).
        let old = r#"{"package":{"spec":"x","id":"europe/testland","version":3},
            "views":[],"next_view":1}"#;
        let w: WorldDoc = serde_json::from_str(old).unwrap();
        assert_eq!(w.patch.package, "europe/testland");
        assert!(w.patch.ops.is_empty());
    }
}
