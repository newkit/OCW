//! The Models workspace's document (spec §14 Phase 16, 16B, stage E4 –
//! edit): one model file of `artwork/3dmodels/` (or any `.glb` / `.gltf`)
//! with its manifest entry, edited by operations with their exact
//! inverses – so every edit of the Models workspace has unlimited undo /
//! redo, merged drags, dirty flags and autosave like the World and Routes
//! documents.
//!
//! **The content** is the model as the Phase 15 reader gives it
//! ([`ocw_assets::model::Model`]: nodes, meshes, materials, `extras`, the
//! copyright), its manifest entry ([`Entry`]: licence, source, author and
//! the kind's keys) and a **stable id per node** ([`NodeId`]): operations,
//! the selection and the views name nodes by id, so a delete, an insert or
//! a re-parent that renumbers the nodes keeps them right. Ids are not
//! counted on: a new node takes the largest id plus one, so undoing an
//! insert gives back the identical document.
//!
//! **The operations** ([`ModelOp`]): a node's transform (the gizmo's drag
//! and the numeric fields; consecutive drags of one node merge into one
//! undo step), its name, its parent (keeping its place in the model or
//! its local transform; the node list is re-ordered so parents stay
//! before their children), nodes inserted and removed (a subtree, with
//! its places, so undo puts them back where they were), a node's
//! `extras`, a material (paint slot, default colour, metallic, roughness,
//! emissive), the asset's `extras` (`extras.ocw`), the copyright, the
//! manifest entry, the whole content replaced (a validator fix that merges
//! materials, a re-import from Blender) and a batch of operations as one
//! step. The helpers on [`ModelDoc`] turn what the user does into them:
//! [`ModelDoc::add_point`] (anchors `anchor_*`, hotspots `hs_*`, levels
//! `lod1` / `lod2` / `billboard`), [`ModelDoc::add_proxy`] (collision
//! proxies `col_box` / `col_cylinder` / `col_capsule` in **both forms**
//! the readers accept – a mesh whose bounds are the shape, or an empty
//! scaled to its half extents; 15H Q9 is open), [`ModelDoc::assign_slot`],
//! [`ModelDoc::fix_op`] (the validator's fixes, 15J), [`ModelDoc::reimport`]
//! (an edited `.glb` back into the document, keeping what Blender drops).
//!
//! **Validators:** the model contract's checker ([`ocw_assets::check`])
//! on every change ([`validator`]). **Saving** goes through the Phase 15
//! writer ([`ocw_assets::glb::write`], deterministic bytes) with the
//! manifest entry upserted as `model import` does ([`ModelDoc::save_plan`]),
//! and the structural diff against the file as it was opened
//! ([`ocw_assets::diff`], `model diff`) to show before saving.

use std::collections::BTreeSet;

use glam::{Quat, Vec3};
use ocw_assets::check::{self, Fix};
use ocw_assets::diff::{self, ModelDiff};
use ocw_assets::kinds::Kind;
use ocw_assets::manifest::{Entry, PHYSICS_KEYS};
use ocw_assets::model::{Material, Model, Node, Primitive};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::document::{DocumentKind, EditError};
use crate::validate::{Finding, Severity, Validator};

/// A node's id within a model document (stable across edits).
pub type NodeId = u64;

/// A node's local transform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Trs {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Trs {
    pub const IDENTITY: Trs = Trs {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn of(n: &Node) -> Self {
        Self {
            translation: n.translation,
            rotation: n.rotation,
            scale: n.scale,
        }
    }

    fn set(self, n: &mut Node) {
        n.translation = self.translation;
        n.rotation = self.rotation;
        n.scale = self.scale;
    }

    fn is_finite(&self) -> bool {
        self.translation.is_finite() && self.rotation.is_finite() && self.scale.is_finite()
    }

    pub fn affine(&self) -> glam::Affine3A {
        glam::Affine3A::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// A node with its place: inserted at `index` (of the node list after the
/// insertion), under `parent` (`node.parent` is ignored).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    pub index: usize,
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub node: Node,
}

/// An edit of a model document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ModelOp {
    /// A node's local transform (the gizmo, the numeric fields).
    Transform {
        node: NodeId,
        trs: Trs,
    },
    Rename {
        node: NodeId,
        name: String,
    },
    /// A node under another parent (`None`: a root) with this local
    /// transform; `order`: the node list's order afterwards (by id; `None`:
    /// the node and what is below it move just behind the new parent when
    /// they would come before it).
    Reparent {
        node: NodeId,
        parent: Option<NodeId>,
        trs: Trs,
        order: Option<Vec<NodeId>>,
    },
    /// Nodes put in at their places (ascending).
    Insert(Vec<Placed>),
    /// Nodes taken out (no remaining node may hang below one of them).
    Remove(Vec<NodeId>),
    /// A node's glTF `extras`.
    NodeExtras {
        node: NodeId,
        extras: Value,
    },
    /// Material `index` replaced (name = paint slot, colours).
    Material {
        index: usize,
        material: Material,
    },
    /// The asset's `extras` (`{"ocw": {…}}`).
    Extras(Value),
    /// `asset.copyright`.
    Copyright(String),
    /// The manifest entry.
    Entry(Entry),
    /// The whole content (material merges, a re-import).
    Replace(Box<ModelDoc>),
    /// Several operations as one step.
    Batch {
        label: String,
        ops: Vec<ModelOp>,
    },
}

/// What can be selected in a model document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModelItem {
    Node(NodeId),
    Material(String),
}

/// The Models workspace's document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDoc {
    /// The manifest entry (made from the file's `extras.ocw` or folder
    /// for a file outside the model folder).
    pub entry: Entry,
    /// The entry is the manifest's (its checks apply, and a save writes
    /// it back).
    pub listed: bool,
    pub model: Model,
    /// The nodes' ids, parallel to `model.nodes`.
    pub ids: Vec<NodeId>,
}

/// A point of the contract a user adds (15B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Point {
    /// `anchor_<name>`.
    Anchor(String),
    /// `hs_<slot>`.
    Hotspot(String),
    /// `lod1`, `lod2` or `billboard`.
    Level(String),
}

impl Point {
    pub fn name(&self) -> String {
        match self {
            Point::Anchor(n) => format!("anchor_{}", n.trim_start_matches("anchor_")),
            Point::Hotspot(s) => format!("hs_{}", s.trim_start_matches("hs_")),
            Point::Level(l) => l.clone(),
        }
    }
}

/// The two forms of a collision proxy the readers accept (15H Q9 is open,
/// both stay).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyForm {
    /// A mesh whose bounds are the shape (node scale 1).
    Mesh,
    /// An empty: the unit shape (half extents 1) scaled by the node.
    Empty,
}

impl ProxyForm {
    pub fn name(self) -> &'static str {
        match self {
            ProxyForm::Mesh => "mesh",
            ProxyForm::Empty => "empty",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mesh" => Some(ProxyForm::Mesh),
            "empty" | "scaled" => Some(ProxyForm::Empty),
            _ => None,
        }
    }
}

/// The manifest keys the entry editor offers (13 rules: licence, source,
/// author; the kind's own keys).
pub const ENTRY_KEYS: [&str; 7] = [
    "license", "source", "author", "profile", "species", "variant", "anchor",
];

/// What a save writes and what it changes.
#[derive(Debug, Clone, PartialEq)]
pub struct SavePlan {
    /// The canonical `.glb` (the Phase 15 writer).
    pub bytes: Vec<u8>,
    /// The manifest with the entry upserted (a listed model), else `None`.
    pub manifest: Option<String>,
    /// `model diff` of the file as opened (or last saved) against the
    /// file to be written, the entry included.
    pub diff: ModelDiff,
    /// The contract's errors in the file to be written (saving is still
    /// allowed – a work in progress – but they are shown).
    pub errors: Vec<String>,
}

fn err(msg: impl Into<String>) -> EditError {
    EditError::new(msg)
}

impl ModelDoc {
    /// A document of `model` (as read) with `entry`; the nodes get the ids
    /// 1, 2, ….
    pub fn new(entry: Entry, model: Model, listed: bool) -> Self {
        let ids = (1..=model.nodes.len() as u64).collect();
        Self {
            entry,
            listed,
            model,
            ids,
        }
    }

    pub fn index_of(&self, id: NodeId) -> Option<usize> {
        self.ids.iter().position(|x| *x == id)
    }

    fn idx(&self, id: NodeId) -> Result<usize, EditError> {
        self.index_of(id)
            .ok_or_else(|| err(format!("no node {id}")))
    }

    pub fn id_of(&self, index: usize) -> Option<NodeId> {
        self.ids.get(index).copied()
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        Some(&self.model.nodes[self.index_of(id)?])
    }

    /// The node called `name`.
    pub fn by_name(&self, name: &str) -> Option<NodeId> {
        self.id_of(self.model.node(name)?)
    }

    /// The parent's id of node `id`.
    pub fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        let p = self.node(id)?.parent?;
        self.id_of(p)
    }

    /// The id a new node takes.
    pub fn fresh_id(&self) -> NodeId {
        self.ids.iter().copied().max().unwrap_or(0) + 1
    }

    /// Node `id` and everything below it, in list order.
    pub fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let Some(root) = self.index_of(id) else {
            return Vec::new();
        };
        (0..self.model.nodes.len())
            .filter(|&i| self.model.is_below(i, root))
            .map(|i| self.ids[i])
            .collect()
    }

    /// A free node name: `base`, else `base_2`, `base_3`, ….
    pub fn free_name(&self, base: &str) -> String {
        if self.model.node(base).is_none() {
            return base.into();
        }
        (2..)
            .map(|k| format!("{base}_{k}"))
            .find(|n| self.model.node(n).is_none())
            .expect("a free name")
    }

    // ----- the content's structure (nodes by id) -----

    fn structure(&self) -> Vec<(NodeId, Option<NodeId>, Node)> {
        self.model
            .nodes
            .iter()
            .zip(&self.ids)
            .map(|(n, id)| (*id, n.parent.map(|p| self.ids[p]), n.clone()))
            .collect()
    }

    /// Sets the nodes from a structure; checks ids are unique and parents
    /// come first. Unchanged on an error.
    fn put(&mut self, list: Vec<(NodeId, Option<NodeId>, Node)>) -> Result<(), EditError> {
        let mut at = std::collections::HashMap::new();
        for (i, (id, _, _)) in list.iter().enumerate() {
            if at.insert(*id, i).is_some() {
                return Err(err(format!("node id {id} twice")));
            }
        }
        let mut parents = Vec::with_capacity(list.len());
        for (i, (_, p, n)) in list.iter().enumerate() {
            let pi = match p {
                None => None,
                Some(p) => {
                    let pi = *at
                        .get(p)
                        .ok_or_else(|| err(format!("{}: no parent node {p}", n.name)))?;
                    if pi >= i {
                        return Err(err(format!("{}: its parent comes after it", n.name)));
                    }
                    Some(pi)
                }
            };
            parents.push(pi);
        }
        self.ids = list.iter().map(|(id, _, _)| *id).collect();
        self.model.nodes = list
            .into_iter()
            .zip(parents)
            .map(|((_, _, mut n), p)| {
                n.parent = p;
                n
            })
            .collect();
        Ok(())
    }

    // ----- helpers: what the user does → operations -----

    /// Node `id` to `trs`.
    pub fn transform(&self, id: NodeId, trs: Trs) -> ModelOp {
        ModelOp::Transform { node: id, trs }
    }

    /// Renames node `id` (a name another node has is refused: names are
    /// the interface, 15B).
    pub fn rename(&self, id: NodeId, name: &str) -> Result<ModelOp, EditError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(err("a node needs a name"));
        }
        if let Some(other) = self.by_name(name)
            && other != id
        {
            return Err(err(format!("another node is called `{name}`")));
        }
        self.idx(id)?;
        Ok(ModelOp::Rename {
            node: id,
            name: name.into(),
        })
    }

    /// Node `id` under `parent`; `keep_place`: it stays where it is in the
    /// model (its local transform is worked out), else it keeps its local
    /// transform and moves with its new parent.
    pub fn reparent(
        &self,
        id: NodeId,
        parent: Option<NodeId>,
        keep_place: bool,
    ) -> Result<ModelOp, EditError> {
        let i = self.idx(id)?;
        if let Some(p) = parent {
            let pi = self.idx(p)?;
            if self.model.is_below(pi, i) {
                return Err(err("a node cannot hang below itself"));
            }
        }
        let n = &self.model.nodes[i];
        let trs = if keep_place {
            let world = self.model.world(i);
            let parent_world = match parent {
                Some(p) => self.model.world(self.idx(p)?),
                None => glam::Affine3A::IDENTITY,
            };
            let (scale, rotation, translation) =
                (parent_world.inverse() * world).to_scale_rotation_translation();
            Trs {
                translation,
                rotation: rotation.normalize(),
                scale,
            }
        } else {
            Trs::of(n)
        };
        Ok(ModelOp::Reparent {
            node: id,
            parent,
            trs,
            order: None,
        })
    }

    /// Deletes node `id` and everything below it.
    pub fn delete(&self, id: NodeId) -> Result<ModelOp, EditError> {
        self.idx(id)?;
        Ok(ModelOp::Remove(self.subtree(id)))
    }

    /// Where a new child of `parent` goes: after the parent's last
    /// descendant (the end for a root).
    fn insert_index(&self, parent: Option<NodeId>) -> usize {
        match parent.and_then(|p| self.index_of(p)) {
            Some(pi) => {
                let mut last = pi;
                for i in pi + 1..self.model.nodes.len() {
                    if self.model.is_below(i, pi) {
                        last = i;
                    }
                }
                last + 1
            }
            None => self.model.nodes.len(),
        }
    }

    /// An insert of one new node.
    pub fn add_node(&self, node: Node, parent: Option<NodeId>) -> Result<ModelOp, EditError> {
        if let Some(p) = parent {
            self.idx(p)?;
        }
        if self.model.node(&node.name).is_some() {
            return Err(err(format!("a node `{}` exists", node.name)));
        }
        Ok(ModelOp::Insert(vec![Placed {
            index: self.insert_index(parent),
            id: self.fresh_id(),
            parent,
            node,
        }]))
    }

    /// Adds an anchor, a hotspot or a level of detail (an empty node) at
    /// `at` (in the parent's frame).
    pub fn add_point(
        &self,
        point: &Point,
        parent: Option<NodeId>,
        at: Vec3,
    ) -> Result<ModelOp, EditError> {
        let name = point.name();
        let bare = match point {
            Point::Anchor(_) => name.strip_prefix("anchor_"),
            Point::Hotspot(_) => name.strip_prefix("hs_"),
            Point::Level(_) => Some(name.as_str()),
        };
        if bare.is_none_or(str::is_empty) {
            return Err(err("the point needs a name"));
        }
        if let Point::Level(l) = point
            && !ocw_assets::model::is_far_level(l)
        {
            return Err(err(format!("`{l}`: a level is lod1, lod2 or billboard")));
        }
        self.add_node(Node::new(&name, None, at), parent)
    }

    /// Adds a collision proxy `col_<shape>` with its centre and half
    /// extents in the parent's frame, as a mesh or a scaled empty.
    pub fn add_proxy(
        &self,
        shape: &str,
        form: ProxyForm,
        parent: Option<NodeId>,
        centre: Vec3,
        half: Vec3,
    ) -> Result<ModelOp, EditError> {
        if !check::COLLISION_SHAPES.contains(&shape) {
            return Err(err(format!(
                "`{shape}`: a proxy is a {}",
                check::COLLISION_SHAPES.join(", ")
            )));
        }
        if !(half.is_finite() && half.min_element() > 0.0) {
            return Err(err("a proxy needs positive half extents"));
        }
        let name = self.free_name(&format!("col_{shape}"));
        let mut node = Node::new(&name, None, centre);
        match form {
            ProxyForm::Empty => node.scale = half,
            ProxyForm::Mesh => {
                if self.model.materials.is_empty() {
                    return Err(err("a mesh proxy needs a material in the model"));
                }
                node.primitives.push(proxy_mesh(shape, half));
            }
        }
        self.add_node(node, parent)
    }

    /// Material `index` edited by `f`.
    pub fn set_material(
        &self,
        index: usize,
        f: impl FnOnce(&mut Material),
    ) -> Result<ModelOp, EditError> {
        let mut m = self
            .model
            .materials
            .get(index)
            .cloned()
            .ok_or_else(|| err(format!("no material {index}")))?;
        f(&mut m);
        Ok(ModelOp::Material { index, material: m })
    }

    /// Assigns paint slot `slot` to material `index` (`defaults`: with the
    /// slot's default colour, metallic and roughness). A slot another
    /// material has already merges the two (the validator's rename fix).
    pub fn assign_slot(
        &self,
        index: usize,
        slot: &str,
        defaults: bool,
    ) -> Result<ModelOp, EditError> {
        let s = ocw_assets::slots::get(slot)
            .ok_or_else(|| err(format!("`{slot}` is no paint slot")))?;
        let mat = self
            .model
            .materials
            .get(index)
            .ok_or_else(|| err(format!("no material {index}")))?;
        if self.model.material(s.name).is_some_and(|j| j != index) {
            let mut next = self.clone();
            Fix::RenameMaterial {
                from: mat.name.clone(),
                to: s.name.into(),
            }
            .apply(&mut next.model, &self.entry);
            return Ok(ModelOp::Replace(Box::new(next)));
        }
        self.set_material(index, |m| {
            m.name = s.name.into();
            if defaults {
                let d = ocw_assets::slots::material(s.name);
                m.color = d.color;
                m.metallic = d.metallic;
                m.roughness = d.roughness;
            }
        })
    }

    /// `extras.ocw.<key>` set (`None`: removed). Physics keys are refused
    /// (2.5); `schema`, `kind` and `id` come from the entry (the stamp
    /// fix).
    pub fn set_ocw(&self, key: &str, value: Option<Value>) -> Result<ModelOp, EditError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(err("a key is needed"));
        }
        if PHYSICS_KEYS.contains(&key) {
            return Err(err(format!(
                "`{key}`: cosmetic models carry no physics (2.5)"
            )));
        }
        if matches!(key, "schema" | "kind" | "id") {
            return Err(err(format!(
                "`{key}` is written from the manifest entry (fix: stamp extras.ocw)"
            )));
        }
        let mut extras = if self.model.extras.is_object() {
            self.model.extras.clone()
        } else {
            serde_json::json!({})
        };
        let root = extras.as_object_mut().expect("an object");
        let ocw = root.entry("ocw").or_insert_with(|| serde_json::json!({}));
        if !ocw.is_object() {
            *ocw = serde_json::json!({});
        }
        let o = ocw.as_object_mut().expect("an object");
        match value {
            Some(v) => {
                o.insert(key.into(), v);
            }
            None => {
                o.remove(key);
            }
        }
        Ok(ModelOp::Extras(extras))
    }

    /// The manifest entry's `key` (one of [`ENTRY_KEYS`]) set to `value`
    /// (empty: unset for the optional keys).
    pub fn set_entry(&self, key: &str, value: &str) -> Result<ModelOp, EditError> {
        let mut e = self.entry.clone();
        let opt = |v: &str| (!v.trim().is_empty()).then(|| v.trim().to_owned());
        match key {
            "license" => e.license = value.trim().into(),
            "source" => e.source = value.trim().into(),
            "author" => e.author = value.trim().into(),
            "profile" => e.profile = opt(value),
            "species" => e.species = opt(value),
            "variant" => e.variant = opt(value),
            "anchor" => e.anchor = opt(value),
            _ => {
                return Err(err(format!(
                    "`{key}`: the entry's keys are {}",
                    ENTRY_KEYS.join(", ")
                )));
            }
        }
        Ok(ModelOp::Entry(e))
    }

    /// The operation of a validator fix (15J), `None` when it changes
    /// nothing now.
    pub fn fix_op(&self, fix: &Fix) -> Option<ModelOp> {
        match fix {
            Fix::RenameNode { from, to } => {
                let id = self.by_name(from)?;
                self.rename(id, to).ok()
            }
            Fix::SetCopyright { text } => {
                (self.model.copyright != *text).then(|| ModelOp::Copyright(text.clone()))
            }
            Fix::SetManifest { key, value } => self.set_entry(key, value).ok(),
            Fix::StampExtras => {
                let mut m = self.model.clone();
                fix.apply(&mut m, &self.entry);
                (m.extras != self.model.extras).then_some(ModelOp::Extras(m.extras))
            }
            Fix::RenameMaterial { .. } | Fix::RemoveMaterial { .. } => {
                let mut next = self.clone();
                if !fix.apply(&mut next.model, &self.entry) || next == *self {
                    return None;
                }
                Some(ModelOp::Replace(Box::new(next)))
            }
        }
    }

    /// The fixes of all findings that offer one, as one step (`None`:
    /// nothing to fix).
    pub fn fix_all_op(&self, findings: &[check::Finding]) -> Option<ModelOp> {
        let mut next = self.clone();
        let mut ops = Vec::new();
        for f in findings {
            let Some(fix) = &f.fix else { continue };
            if let Some(op) = next.fix_op(fix)
                && let Ok(_inv) = next.apply(&op)
            {
                ops.push(op);
            }
        }
        (!ops.is_empty()).then(|| ModelOp::Batch {
            label: format!("Apply {} fixes", ops.len()),
            ops,
        })
    }

    /// The model's own findings (the manifest entry's too when listed);
    /// `bytes`: the file's size.
    pub fn findings(&self, bytes: usize) -> Vec<check::Finding> {
        if self.listed {
            check::check(&self.entry, &self.model, bytes)
        } else {
            check::check_model(&self.entry, &self.model, bytes)
        }
    }

    /// An edited file read back (`model import` of a `.glb` from Blender
    /// or any tool) as the new content: normalised as the import does,
    /// with what Blender drops kept from the document (asset extras, the
    /// copyright), and the contract points of the document (anchors,
    /// hotspots, proxies) the new file lacks put back under their parents
    /// where those are still there. Nodes keep their ids by name. Returns
    /// the operation and the notes.
    pub fn reimport(&self, mut m: Model) -> (ModelOp, Vec<String>) {
        let mut notes = ocw_assets::import::normalise(&mut m);
        notes.extend(ocw_assets::import::keep_from_previous(&mut m, &self.model));
        // The studio's points the new file lacks.
        for (i, n) in self.model.nodes.iter().enumerate() {
            let point = n.name.starts_with("anchor_")
                || n.name.starts_with("hs_")
                || n.name.starts_with("col_");
            if !point || m.node(&n.name).is_some() {
                continue;
            }
            let parent = match n.parent {
                None => None,
                Some(p) => match m.node(&self.model.nodes[p].name) {
                    Some(q) => Some(q),
                    None => continue,
                },
            };
            let mut keep = self.model.nodes[i].clone();
            keep.parent = parent;
            // Under the parent's last descendant, so parents stay first.
            let at = match parent {
                Some(q) => {
                    let mut last = q;
                    for j in q + 1..m.nodes.len() {
                        if m.is_below(j, q) {
                            last = j;
                        }
                    }
                    last + 1
                }
                None => m.nodes.len(),
            };
            for x in &mut m.nodes {
                if let Some(p) = x.parent.as_mut()
                    && *p >= at
                {
                    *p += 1;
                }
            }
            m.nodes.insert(at, keep);
            notes.push(format!("{} kept from the document", n.name));
        }
        let mut next_id = self.fresh_id();
        let ids = m
            .nodes
            .iter()
            .map(|n| match self.by_name(&n.name) {
                Some(id) => id,
                None => {
                    next_id += 1;
                    next_id - 1
                }
            })
            .collect::<Vec<_>>();
        // Duplicate names (a file with two nodes of one name): fresh ids.
        let mut seen = BTreeSet::new();
        let ids = ids
            .into_iter()
            .map(|id| {
                if seen.insert(id) {
                    id
                } else {
                    next_id += 1;
                    next_id - 1
                }
            })
            .collect();
        let next = ModelDoc {
            entry: self.entry.clone(),
            listed: self.listed,
            model: m,
            ids,
        };
        (ModelOp::Replace(Box::new(next)), notes)
    }

    /// What saving writes: the canonical file, the manifest (`manifest`:
    /// the folder's manifest text, for a listed model) and the diff
    /// against `before` (the model and entry as opened or last saved).
    pub fn save_plan(
        &self,
        before: Option<(&Model, &Entry)>,
        manifest: Option<&str>,
    ) -> Result<SavePlan, String> {
        let bytes = ocw_assets::glb::write(&self.model);
        let canonical = ocw_assets::glb::read(&bytes).map_err(|e| e.to_string())?;
        let mut d = match before {
            Some((m, _)) => diff::diff(m, &canonical),
            None => diff::diff(&Model::default(), &canonical),
        };
        let strip = |e: &Entry| {
            let mut e = e.clone();
            e.file.clear();
            e
        };
        d.changes.extend(diff::diff_entries(
            before.map(|(_, e)| strip(e)).as_ref(),
            Some(&strip(&self.entry)),
        ));
        let manifest = match (self.listed, manifest) {
            (true, Some(old)) => Some(
                ocw_assets::import::upsert_manifest(old, &self.entry).map_err(|e| e.join("; "))?,
            ),
            _ => None,
        };
        let errors = self
            .findings(bytes.len())
            .iter()
            .filter(|f| f.is_error())
            .map(|f| f.line())
            .collect();
        Ok(SavePlan {
            bytes,
            manifest,
            diff: d,
            errors,
        })
    }
}

/// The contract's node names for a kind (the rename picker, 15B): the
/// required and optional nodes, the hotspots (bikes), the levels and the
/// proxy shapes.
pub fn contract_names(kind: Kind) -> Vec<String> {
    use ocw_assets::pose::{bike_nodes, nodes};
    let mut out: Vec<String> = Vec::new();
    match kind {
        Kind::Bike => {
            out.extend(bike_nodes::REQUIRED.iter().map(|s| s.to_string()));
            out.extend(bike_nodes::HOTSPOTS.iter().map(|s| s.to_string()));
        }
        Kind::Rider => out.extend(nodes::REQUIRED.iter().map(|s| s.to_string())),
        _ => {}
    }
    out.push("lod1".into());
    if kind == Kind::Plant {
        out.push("lod2".into());
        out.push("billboard".into());
    }
    out.extend(check::COLLISION_SHAPES.iter().map(|s| format!("col_{s}")));
    out
}

/// A closed proxy mesh with half extents `half` about the origin: a box,
/// or a 12-sided prism for a cylinder or a capsule (only its bounds
/// matter; proxies are never drawn).
pub fn proxy_mesh(shape: &str, half: Vec3) -> Primitive {
    let mut p = Primitive::default();
    let quad = |p: &mut Primitive, c: [Vec3; 4], n: Vec3| {
        let base = p.positions.len() as u32;
        for v in c {
            p.positions.push(v.to_array());
            p.normals.push(n.to_array());
        }
        p.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    if shape == "box" {
        let h = half;
        for axis in 0..3 {
            for sign in [-1.0f32, 1.0] {
                let mut n = Vec3::ZERO;
                n[axis] = sign;
                let (u, v) = match axis {
                    0 => (Vec3::Y, Vec3::Z),
                    1 => (Vec3::Z, Vec3::X),
                    _ => (Vec3::X, Vec3::Y),
                };
                let c = n * h;
                let (du, dv) = (u * h, v * h);
                let mut corners = [c - du - dv, c + du - dv, c + du + dv, c - du + dv];
                if sign < 0.0 {
                    corners.reverse();
                }
                quad(&mut p, corners, n);
            }
        }
        return p;
    }
    const SIDES: u32 = 12;
    let ring = |k: u32| {
        let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
        Vec3::new(a.cos() * half.x, 0.0, a.sin() * half.z)
    };
    for k in 0..SIDES {
        let (a, b) = (ring(k), ring(k + 1));
        let n = ((a + b) * 0.5).normalize_or_zero();
        let up = Vec3::Y * half.y;
        quad(&mut p, [a - up, a + up, b + up, b - up], n);
    }
    for (y, n) in [(half.y, Vec3::Y), (-half.y, Vec3::NEG_Y)] {
        let base = p.positions.len() as u32;
        p.positions.push([0.0, y, 0.0]);
        p.normals.push(n.to_array());
        for k in 0..SIDES {
            let r = ring(k) + Vec3::Y * y;
            p.positions.push(r.to_array());
            p.normals.push(n.to_array());
        }
        for k in 0..SIDES {
            let (i, j) = (base + 1 + k, base + 1 + (k + 1) % SIDES);
            if y > 0.0 {
                p.indices.extend([base, j, i]);
            } else {
                p.indices.extend([base, i, j]);
            }
        }
    }
    p
}

impl DocumentKind for ModelDoc {
    type Op = ModelOp;
    type Item = ModelItem;
    const KIND: &'static str = "model";

    fn apply(&mut self, op: &ModelOp) -> Result<ModelOp, EditError> {
        match op {
            ModelOp::Transform { node, trs } => {
                if !trs.is_finite() {
                    return Err(err("the transform is not finite"));
                }
                let i = self.idx(*node)?;
                let old = Trs::of(&self.model.nodes[i]);
                trs.set(&mut self.model.nodes[i]);
                Ok(ModelOp::Transform {
                    node: *node,
                    trs: old,
                })
            }
            ModelOp::Rename { node, name } => {
                let i = self.idx(*node)?;
                let old = std::mem::replace(&mut self.model.nodes[i].name, name.clone());
                Ok(ModelOp::Rename {
                    node: *node,
                    name: old,
                })
            }
            ModelOp::NodeExtras { node, extras } => {
                let i = self.idx(*node)?;
                let old = std::mem::replace(&mut self.model.nodes[i].extras, extras.clone());
                Ok(ModelOp::NodeExtras {
                    node: *node,
                    extras: old,
                })
            }
            ModelOp::Reparent {
                node,
                parent,
                trs,
                order,
            } => {
                if !trs.is_finite() {
                    return Err(err("the transform is not finite"));
                }
                let i = self.idx(*node)?;
                if let Some(p) = parent {
                    let pi = self.idx(*p)?;
                    if self.model.is_below(pi, i) {
                        return Err(err("a node cannot hang below itself"));
                    }
                }
                let old_parent = self.parent_of(*node);
                let old_trs = Trs::of(&self.model.nodes[i]);
                let old_order = self.ids.clone();
                let mut list = self.structure();
                list[i].1 = *parent;
                trs.set(&mut list[i].2);
                let list = match order {
                    Some(order) => {
                        if order.len() != list.len() {
                            return Err(err("the order names other nodes"));
                        }
                        let mut out = Vec::with_capacity(list.len());
                        let mut taken = vec![false; list.len()];
                        for id in order {
                            let j = list
                                .iter()
                                .position(|(x, ..)| x == id)
                                .ok_or_else(|| err(format!("the order names no node {id}")))?;
                            if std::mem::replace(&mut taken[j], true) {
                                return Err(err(format!("the order names node {id} twice")));
                            }
                            out.push(list[j].clone());
                        }
                        out
                    }
                    None => behind_parent(list, *node, *parent),
                };
                self.put(list)?;
                Ok(ModelOp::Reparent {
                    node: *node,
                    parent: old_parent,
                    trs: old_trs,
                    order: Some(old_order),
                })
            }
            ModelOp::Insert(items) => {
                let mut list = self.structure();
                let mut ids: BTreeSet<NodeId> = self.ids.iter().copied().collect();
                let mut sorted: Vec<&Placed> = items.iter().collect();
                sorted.sort_by_key(|p| p.index);
                for p in sorted {
                    if !ids.insert(p.id) {
                        return Err(err(format!("node id {} is taken", p.id)));
                    }
                    if p.index > list.len() {
                        return Err(err(format!("no place {} for {}", p.index, p.node.name)));
                    }
                    list.insert(p.index, (p.id, p.parent, p.node.clone()));
                }
                self.put(list)?;
                Ok(ModelOp::Remove(items.iter().map(|p| p.id).collect()))
            }
            ModelOp::Remove(gone) => {
                let set: BTreeSet<NodeId> = gone.iter().copied().collect();
                for id in &set {
                    self.idx(*id)?;
                }
                let list = self.structure();
                if let Some((_, _, n)) = list
                    .iter()
                    .find(|(id, p, _)| !set.contains(id) && p.is_some_and(|p| set.contains(&p)))
                {
                    return Err(err(format!("{} hangs below a removed node", n.name)));
                }
                let mut placed = Vec::new();
                let mut keep = Vec::new();
                for (index, (id, parent, node)) in list.into_iter().enumerate() {
                    if set.contains(&id) {
                        placed.push(Placed {
                            index,
                            id,
                            parent,
                            node,
                        });
                    } else {
                        keep.push((id, parent, node));
                    }
                }
                self.put(keep)?;
                Ok(ModelOp::Insert(placed))
            }
            ModelOp::Material { index, material } => {
                let i = *index;
                if i >= self.model.materials.len() {
                    return Err(err(format!("no material {i}")));
                }
                if self
                    .model
                    .materials
                    .iter()
                    .enumerate()
                    .any(|(j, m)| j != i && m.name == material.name)
                {
                    return Err(err(format!(
                        "another material is `{}` (merge them with the slot picker)",
                        material.name
                    )));
                }
                let old = std::mem::replace(&mut self.model.materials[i], material.clone());
                Ok(ModelOp::Material {
                    index: i,
                    material: old,
                })
            }
            ModelOp::Extras(v) => {
                let old = std::mem::replace(&mut self.model.extras, v.clone());
                Ok(ModelOp::Extras(old))
            }
            ModelOp::Copyright(c) => {
                let old = std::mem::replace(&mut self.model.copyright, c.clone());
                Ok(ModelOp::Copyright(old))
            }
            ModelOp::Entry(e) => {
                if e.file != self.entry.file || e.kind != self.entry.kind {
                    return Err(err("the entry's file and kind stay"));
                }
                let old = std::mem::replace(&mut self.entry, e.clone());
                Ok(ModelOp::Entry(old))
            }
            ModelOp::Replace(next) => {
                if next.ids.len() != next.model.nodes.len() {
                    return Err(err("the ids do not match the nodes"));
                }
                let old = std::mem::replace(self, (**next).clone());
                Ok(ModelOp::Replace(Box::new(old)))
            }
            ModelOp::Batch { label, ops } => {
                let mut inverses = Vec::with_capacity(ops.len());
                for op in ops {
                    match self.apply(op) {
                        Ok(inv) => inverses.push(inv),
                        Err(e) => {
                            for inv in inverses.iter().rev() {
                                let _ = self.apply(inv);
                            }
                            return Err(e);
                        }
                    }
                }
                inverses.reverse();
                Ok(ModelOp::Batch {
                    label: label.clone(),
                    ops: inverses,
                })
            }
        }
    }

    fn label(op: &ModelOp) -> String {
        match op {
            ModelOp::Transform { .. } => "Move node".into(),
            ModelOp::Rename { .. } => "Rename node".into(),
            ModelOp::Reparent { .. } => "Re-parent node".into(),
            ModelOp::Insert(items) => match items.as_slice() {
                [p] => format!("Add {}", p.node.name),
                _ => format!("Add {} nodes", items.len()),
            },
            ModelOp::Remove(ids) => match ids.len() {
                1 => "Delete node".into(),
                n => format!("Delete {n} nodes"),
            },
            ModelOp::NodeExtras { .. } => "Edit node extras".into(),
            ModelOp::Material { .. } => "Edit material".into(),
            ModelOp::Extras(_) => "Edit extras.ocw".into(),
            ModelOp::Copyright(_) => "Set copyright".into(),
            ModelOp::Entry(_) => "Edit manifest entry".into(),
            ModelOp::Replace(_) => "Replace model".into(),
            ModelOp::Batch { label, .. } => label.clone(),
        }
    }

    fn merges(prev: &ModelOp, next: &ModelOp) -> bool {
        match (prev, next) {
            (ModelOp::Transform { node: a, .. }, ModelOp::Transform { node: b, .. })
            | (ModelOp::Rename { node: a, .. }, ModelOp::Rename { node: b, .. })
            | (ModelOp::NodeExtras { node: a, .. }, ModelOp::NodeExtras { node: b, .. }) => a == b,
            (ModelOp::Material { index: a, .. }, ModelOp::Material { index: b, .. }) => a == b,
            (ModelOp::Extras(_), ModelOp::Extras(_))
            | (ModelOp::Copyright(_), ModelOp::Copyright(_))
            | (ModelOp::Entry(_), ModelOp::Entry(_)) => true,
            _ => false,
        }
    }

    fn contains(&self, item: &ModelItem) -> bool {
        match item {
            ModelItem::Node(id) => self.ids.contains(id),
            ModelItem::Material(n) => self.model.material(n).is_some(),
        }
    }
}

/// The list with `node` (and what is below it) moved just behind its new
/// parent when it would come before it; the rest keeps its order.
fn behind_parent(
    list: Vec<(NodeId, Option<NodeId>, Node)>,
    node: NodeId,
    parent: Option<NodeId>,
) -> Vec<(NodeId, Option<NodeId>, Node)> {
    let pos = |l: &[(NodeId, Option<NodeId>, Node)], id: NodeId| l.iter().position(|x| x.0 == id);
    let (Some(p), Some(n)) = (parent.and_then(|p| pos(&list, p)), pos(&list, node)) else {
        return list;
    };
    if p < n {
        return list;
    }
    // The subtree by the new parents.
    let mut moving: BTreeSet<NodeId> = BTreeSet::from([node]);
    loop {
        let before = moving.len();
        for (id, par, _) in &list {
            if par.is_some_and(|q| moving.contains(&q)) {
                moving.insert(*id);
            }
        }
        if moving.len() == before {
            break;
        }
    }
    let (sub, mut rest): (Vec<_>, Vec<_>) = list.into_iter().partition(|x| moving.contains(&x.0));
    let at = pos(&rest, parent.expect("checked")).expect("the parent stays") + 1;
    rest.splice(at..at, sub);
    rest
}

/// The validator of model documents: the contract's checks (15J) on every
/// change, as the document's findings (the file's size from the writer).
pub fn validator() -> Validator<ModelDoc> {
    Box::new(|d: &ModelDoc| {
        let bytes = ocw_assets::glb::write(&d.model).len();
        d.findings(bytes)
            .into_iter()
            .map(|f| Finding {
                severity: if f.is_error() {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                message: f.line(),
                target: match &f.target {
                    check::Target::Node(n) | check::Target::Material(n) => Some(n.clone()),
                    check::Target::Manifest(k) => Some(format!("manifest.{k}")),
                    check::Target::File => None,
                },
            })
            .collect()
    })
}

#[cfg(test)]
mod tests;
