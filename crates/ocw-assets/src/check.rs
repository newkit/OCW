//! The extended model validator (spec §14 Phase 15, M2; 15B / 15D
//! `validate`): every check of the model contract as a machine-readable
//! [`Finding`] – a stable code, the severity, the node or material it is
//! about and, where the fix is clear, a [`Fix`] (rename `wheel_front.001`
//! to `wheel_front`, drop an unused material, stamp `extras.ocw`). The
//! manifest validator ([`crate::manifest::validate`]) fails on the errors;
//! the warnings are listed. OCW Studio's Models workspace (spec §14 Phase
//! 16, 16B) shows the same findings live, with the fixes as hints.
//!
//! What is checked, per manifest entry and model:
//! - the entry: `license`, `source`, `author` present, the licence on the
//!   accepted list (§13), an attribution licence with its source URL, the
//!   file name (15H), the kind's own keys (`profile`, `species`, `anchor`);
//! - the file: `asset.copyright`, `asset.extras.ocw` (schema, kind, id, no
//!   physics), materials are paint slots (and used), node names unique and
//!   without Blender's `.001` suffixes;
//! - the contract names: required nodes and the rig per kind, hotspots
//!   `hs_<slot>` of the workshop's slots (bikes only), anchors `anchor_*`
//!   empty, collision proxies `col_box*` / `col_cylinder*` /
//!   `col_capsule*`, a prop's proxy, `lod1` where the kind needs it;
//! - the budgets per kind (near and `lod1`), the file size;
//! - the bounds: the origin of the kind (on the ground for bikes, props,
//!   plants and creatures), a model lying on its back
//!   (exported without "+Y up"), a model in centimetres.

use std::collections::BTreeSet;

use glam::Vec3;
use serde::Serialize;

use crate::kinds::{self, Kind};
use crate::manifest::{Entry, PHYSICS_KEYS};
use crate::model::{Model, is_far_level};
use crate::pose::{BikeRig, RiderRig, bike_nodes, nodes};
use crate::slots;

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The file breaks the contract: the validator fails.
    Error,
    /// Worth a look; the file still loads and passes.
    Warning,
}

/// What a finding is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "name", rename_all = "lowercase")]
pub enum Target {
    /// The whole file.
    File,
    /// The manifest entry's key.
    Manifest(String),
    Node(String),
    Material(String),
}

/// A fix that is clear enough to name (and, in the importer and later the
/// studio's edit stage E4, to apply).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "fix", rename_all = "snake_case")]
pub enum Fix {
    RenameNode {
        from: String,
        to: String,
    },
    RenameMaterial {
        from: String,
        to: String,
    },
    RemoveMaterial {
        name: String,
    },
    /// Write `asset.extras.ocw = { schema, kind, id }` from the entry
    /// (keeping the other keys).
    StampExtras,
    SetCopyright {
        text: String,
    },
    /// Set a manifest key.
    SetManifest {
        key: String,
        value: String,
    },
}

impl Fix {
    /// The fix in a few words ("rename `wheel_front.001` to `wheel_front`").
    pub fn describe(&self) -> String {
        match self {
            Fix::RenameNode { from, to } => format!("rename node `{from}` to `{to}`"),
            Fix::RenameMaterial { from, to } => format!("rename material `{from}` to `{to}`"),
            Fix::RemoveMaterial { name } => format!("remove the unused material `{name}`"),
            Fix::StampExtras => "write extras.ocw (schema, kind, id) from the manifest".into(),
            Fix::SetCopyright { text } => format!("set asset.copyright to \"{text}\""),
            Fix::SetManifest { key, value } => format!("set `{key} = \"{value}\"` in the manifest"),
        }
    }

    /// Applies the fix to the model (manifest fixes do nothing here);
    /// whether it changed anything.
    pub fn apply(&self, m: &mut Model, e: &Entry) -> bool {
        match self {
            Fix::RenameNode { from, to } => {
                if m.node(to).is_some() {
                    return false;
                }
                match m.node(from) {
                    Some(i) => {
                        m.nodes[i].name.clone_from(to);
                        true
                    }
                    None => false,
                }
            }
            Fix::RenameMaterial { from, to } => {
                let Some(i) = m.material(from) else {
                    return false;
                };
                match m.material(to) {
                    // Merge into the existing slot material.
                    Some(j) => {
                        for n in &mut m.nodes {
                            for p in &mut n.primitives {
                                if p.material == i {
                                    p.material = j;
                                }
                            }
                        }
                        remove_material(m, i);
                    }
                    None => m.materials[i].name.clone_from(to),
                }
                true
            }
            Fix::RemoveMaterial { name } => match m.material(name) {
                Some(i) if !material_used(m, i) => {
                    remove_material(m, i);
                    true
                }
                _ => false,
            },
            Fix::StampExtras => {
                let mut more = m
                    .ocw()
                    .cloned()
                    .map(serde_json::Value::Object)
                    .unwrap_or_else(|| serde_json::json!({}));
                if let Some(o) = more.as_object_mut() {
                    o.retain(|k, _| {
                        !PHYSICS_KEYS.contains(&k.as_str())
                            && !matches!(k.as_str(), "schema" | "kind" | "id")
                    });
                }
                let copyright = std::mem::take(&mut m.copyright);
                let generator = std::mem::take(&mut m.generator);
                crate::export::stamp(m, e, more);
                m.copyright = copyright;
                m.generator = generator;
                true
            }
            Fix::SetCopyright { text } => {
                m.copyright.clone_from(text);
                true
            }
            Fix::SetManifest { .. } => false,
        }
    }
}

fn material_used(m: &Model, i: usize) -> bool {
    m.nodes
        .iter()
        .any(|n| n.primitives.iter().any(|p| p.material == i))
}

/// Removes material `i` (unused) and renumbers the primitives.
fn remove_material(m: &mut Model, i: usize) {
    m.materials.remove(i);
    for n in &mut m.nodes {
        for p in &mut n.primitives {
            if p.material > i {
                p.material -= 1;
            }
        }
    }
}

/// One finding of the validator.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// A stable identifier (`node.blender-suffix`, `budget.triangles`, …).
    pub code: &'static str,
    /// The model file (its path in the model folder).
    pub file: String,
    pub target: Target,
    /// The message, without the file.
    pub message: String,
    pub fix: Option<Fix>,
}

impl Finding {
    /// `FILE: message`, as the validator has always printed it.
    pub fn line(&self) -> String {
        if self.file.is_empty() {
            self.message.clone()
        } else {
            format!("{}: {}", self.file, self.message)
        }
    }

    /// The finding as one JSON object (the machine-readable form).
    pub fn json(&self) -> String {
        serde_json::to_string(self).expect("JSON")
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{s}: {}", self.line())?;
        if let Some(fix) = &self.fix {
            write!(f, " (fix: {})", fix.describe())?;
        }
        Ok(())
    }
}

struct Out<'a> {
    file: &'a str,
    list: Vec<Finding>,
}

impl Out<'_> {
    fn push(
        &mut self,
        severity: Severity,
        code: &'static str,
        target: Target,
        message: impl Into<String>,
        fix: Option<Fix>,
    ) {
        self.list.push(Finding {
            severity,
            code,
            file: self.file.to_owned(),
            target,
            message: message.into(),
            fix,
        });
    }

    fn error(&mut self, code: &'static str, target: Target, message: impl Into<String>) {
        self.push(Severity::Error, code, target, message, None);
    }

    fn warn(&mut self, code: &'static str, target: Target, message: impl Into<String>) {
        self.push(Severity::Warning, code, target, message, None);
    }
}

/// The name without a Blender duplicate suffix (`wheel_front.001` →
/// `wheel_front`); `None` when it has none.
pub fn strip_blender_suffix(name: &str) -> Option<&str> {
    let (base, num) = name.rsplit_once('.')?;
    (!base.is_empty() && num.len() == 3 && num.bytes().all(|b| b.is_ascii_digit())).then_some(base)
}

/// The paint slot a material name means, when it is not one itself but
/// close (Blender's suffix, upper case, spaces): `Frame.001` → `frame`.
pub fn slot_for(name: &str) -> Option<&'static str> {
    let base = strip_blender_suffix(name).unwrap_or(name);
    let norm = base.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    slots::get(&norm).map(|s| s.name)
}

/// The collision proxy shapes of `col_<shape>` (15B, Phase 12).
pub const COLLISION_SHAPES: [&str; 3] = ["box", "cylinder", "capsule"];

/// The shape of a collision proxy node (`col_box`, `col_cylinder_post`).
pub fn collision_shape(name: &str) -> Option<&'static str> {
    let rest = name.strip_prefix("col_")?;
    COLLISION_SHAPES
        .into_iter()
        .find(|s| rest == *s || rest.strip_prefix(s).is_some_and(|r| r.starts_with('_')))
}

/// Kinds whose origin is on the ground (15B).
fn stands_on_ground(k: Kind) -> bool {
    matches!(k, Kind::Bike | Kind::Prop | Kind::Plant | Kind::Creature)
}

/// How far a grounded model may reach below or float above its origin
/// (m).
pub const GROUND_TOLERANCE_M: f32 = 0.05;

/// Whether the bounds look like an upright model lying on its back
/// (exported without "+Y up"): kinds that are taller than deep (riders,
/// plants) much flatter in Y than in Z.
pub fn lies_on_its_back(kind: Kind, lo: Vec3, hi: Vec3) -> bool {
    let size = hi - lo;
    matches!(kind, Kind::Rider | Kind::Plant) && size.y < 0.5 * size.z && size.z > 0.05
}

/// Checks a manifest entry alone.
pub fn check_entry(e: &Entry) -> Vec<Finding> {
    let mut out = Out {
        file: &e.file,
        list: Vec::new(),
    };
    for (key, value) in [
        ("license", &e.license),
        ("source", &e.source),
        ("author", &e.author),
    ] {
        if value.trim().is_empty() {
            out.error(
                "entry.missing-field",
                Target::Manifest(key.into()),
                format!("no `{key}` – every asset needs a licence entry (spec §13)"),
            );
        }
    }
    if !e.license.trim().is_empty() {
        match crate::licence::check(&e.license) {
            Err(why) => out.error(
                "entry.licence",
                Target::Manifest("license".into()),
                format!("licence `{}`: {why}", e.license),
            ),
            Ok(crate::licence::Obligation::Attribution)
                if !e.source.contains("://") && !e.source.trim().is_empty() =>
            {
                out.warn(
                    "entry.attribution-source",
                    Target::Manifest("source".into()),
                    format!(
                        "licence `{}` asks for attribution: give the source URL in `source` (15C, Q3)",
                        e.license
                    ),
                );
            }
            Ok(_) => {}
        }
    }
    if let Err(why) = kinds::check_name(&e.file, e.kind) {
        out.error("name.pattern", Target::File, why);
    }
    match (e.kind, &e.profile) {
        (Kind::Bike, None) => out.error(
            "entry.profile-missing",
            Target::Manifest("profile".into()),
            "a bike needs its `profile`",
        ),
        (Kind::Bike, Some(p)) if ocw_core::ProfileId::parse(p).is_none() => out.error(
            "entry.profile-unknown",
            Target::Manifest("profile".into()),
            format!("unknown physics profile `{p}`"),
        ),
        (Kind::Bike, Some(_)) | (_, None) => {}
        (k, Some(_)) => out.error(
            "entry.profile-unexpected",
            Target::Manifest("profile".into()),
            format!("a {} has no `profile`", k.name()),
        ),
    }
    if e.kind == Kind::Plant && e.species.is_none() {
        out.error(
            "entry.species-missing",
            Target::Manifest("species".into()),
            "a plant needs its `species`",
        );
    }
    match (e.kind, &e.anchor) {
        (Kind::Wearable, None) => out.error(
            "entry.anchor-missing",
            Target::Manifest("anchor".into()),
            "a wearable needs its `anchor`",
        ),
        (Kind::Wearable, Some(a)) if !nodes::REQUIRED.contains(&a.as_str()) => out.error(
            "entry.anchor-unknown",
            Target::Manifest("anchor".into()),
            format!("anchor `{a}` is no node of the rider (`head`, `torso`, `hand_l`, …)"),
        ),
        _ => {}
    }
    out.list
}

/// Checks a model against its entry's kind (the file's own findings;
/// [`check_entry`] has the entry's). `bytes`: the file's size.
pub fn check_model(e: &Entry, m: &Model, bytes: usize) -> Vec<Finding> {
    let mut out = Out {
        file: &e.file,
        list: Vec::new(),
    };
    let kind = e.kind;
    if m.copyright.trim().is_empty() {
        let fix = (e.license == crate::export::LICENSE).then(|| Fix::SetCopyright {
            text: crate::COPYRIGHT.into(),
        });
        out.push(
            Severity::Error,
            "model.copyright",
            Target::File,
            "no asset.copyright (the licence inside the file)",
            fix,
        );
    }
    // Materials: paint slots, used.
    for (i, mat) in m.materials.iter().enumerate() {
        if slots::get(&mat.name).is_none() {
            let fix = slot_for(&mat.name).map(|to| Fix::RenameMaterial {
                from: mat.name.clone(),
                to: to.into(),
            });
            out.push(
                Severity::Error,
                "material.not-slot",
                Target::Material(mat.name.clone()),
                format!("material `{}` is no paint slot", mat.name),
                fix,
            );
        }
        if !material_used(m, i) {
            out.push(
                Severity::Warning,
                "material.unused",
                Target::Material(mat.name.clone()),
                format!("material `{}` is not used by any mesh", mat.name),
                Some(Fix::RemoveMaterial {
                    name: mat.name.clone(),
                }),
            );
        }
    }
    // Node names.
    let mut names = BTreeSet::new();
    for n in &m.nodes {
        if n.name.is_empty() {
            out.warn(
                "node.unnamed",
                Target::Node(String::new()),
                "a node without a name (names are the interface, 15B)",
            );
        } else if !names.insert(n.name.as_str()) {
            out.error(
                "node.duplicate",
                Target::Node(n.name.clone()),
                format!("node name `{}` twice", n.name),
            );
        }
    }
    for n in &m.nodes {
        if let Some(base) = strip_blender_suffix(&n.name) {
            let free = m.node(base).is_none();
            out.push(
                Severity::Warning,
                "node.blender-suffix",
                Target::Node(n.name.clone()),
                format!(
                    "node `{}` has Blender's duplicate suffix{}",
                    n.name,
                    if free {
                        String::new()
                    } else {
                        format!(" (and `{base}` exists)")
                    }
                ),
                free.then(|| Fix::RenameNode {
                    from: n.name.clone(),
                    to: base.into(),
                }),
            );
        }
    }
    // The asset's own metadata (15B): `extras.ocw = { schema, kind, id }`.
    match m.ocw() {
        None => out.push(
            Severity::Error,
            "extras.missing",
            Target::File,
            "no asset.extras.ocw (schema, kind, id)",
            Some(Fix::StampExtras),
        ),
        Some(o) => {
            if o.get("schema").and_then(serde_json::Value::as_u64) != Some(kinds::SCHEMA) {
                out.push(
                    Severity::Error,
                    "extras.schema",
                    Target::File,
                    format!("extras.ocw.schema is not {}", kinds::SCHEMA),
                    Some(Fix::StampExtras),
                );
            }
            if o.get("kind").and_then(serde_json::Value::as_str) != Some(kind.name()) {
                out.push(
                    Severity::Error,
                    "extras.kind",
                    Target::File,
                    format!("extras.ocw.kind is not `{}`", kind.name()),
                    Some(Fix::StampExtras),
                );
            }
            if o.get("id").and_then(serde_json::Value::as_str) != Some(e.id()) {
                out.push(
                    Severity::Error,
                    "extras.id",
                    Target::File,
                    format!("extras.ocw.id is not `{}`", e.id()),
                    Some(Fix::StampExtras),
                );
            }
            for k in o.keys() {
                if PHYSICS_KEYS.contains(&k.as_str()) {
                    out.push(
                        Severity::Error,
                        "extras.physics",
                        Target::File,
                        format!("extras.ocw.{k} – cosmetic assets carry no physics (spec §2.5)"),
                        Some(Fix::StampExtras),
                    );
                }
            }
        }
    }
    // Levels of detail and budgets.
    let tris = m.triangles();
    let (near, far) = kind.budget();
    let lod1 = m.node(bike_nodes::LOD1);
    if kind.needs_lod1() && lod1.is_none() {
        out.error(
            "lod.missing",
            Target::Node("lod1".into()),
            "missing node `lod1`",
        );
    }
    if kind == Kind::Creature && lod1.is_none() {
        out.warn(
            "lod.missing",
            Target::Node("lod1".into()),
            "a creature should have its far model in `lod1` (15B)",
        );
    }
    if let (Some(l), Some(far)) = (lod1, far) {
        let n = m.triangles_below(Some(l));
        if n == 0 || n > far {
            out.error(
                "lod.budget",
                Target::Node("lod1".into()),
                format!("lod1 has {n} triangles (1…{far})"),
            );
        }
    }
    let missing = |out: &mut Out, list: &[&str]| {
        for n in list {
            if m.node(n).is_none() {
                out.error(
                    "node.missing",
                    Target::Node((*n).into()),
                    format!("missing node `{n}`"),
                );
            }
        }
    };
    match kind {
        Kind::Bike => {
            missing(&mut out, &bike_nodes::REQUIRED);
            missing(&mut out, &bike_nodes::HOTSPOTS);
            if let Err(err) = BikeRig::from_model(m) {
                out.error("rig.invalid", Target::File, err);
            }
            if tris > near {
                out.error(
                    "budget.triangles",
                    Target::File,
                    format!("{tris} triangles (budget {near})"),
                );
            }
        }
        Kind::Rider => {
            missing(&mut out, &nodes::REQUIRED);
            if let Err(err) = RiderRig::from_model(m) {
                out.error("rig.invalid", Target::File, err);
            }
            if tris > near {
                out.error(
                    "budget.triangles",
                    Target::File,
                    format!("{tris} triangles (budget {near})"),
                );
            }
        }
        _ => {
            let own = tris - lod1.map_or(0, |l| m.triangles_below(Some(l)));
            if own == 0 || own > near {
                out.error(
                    "budget.triangles",
                    Target::File,
                    format!("{own} triangles (1…{near})"),
                );
            }
        }
    }
    // The contract names: hotspots, anchors, collision proxies.
    let mut proxies = 0;
    for n in &m.nodes {
        let name = n.name.as_str();
        if let Some(slot) = name.strip_prefix("hs_") {
            if kind != Kind::Bike {
                out.warn(
                    "contract.hotspot-misplaced",
                    Target::Node(name.into()),
                    format!(
                        "hotspot `{name}` in a {}: hotspots are the bike's (§8.2)",
                        kind.name()
                    ),
                );
            } else if !bike_nodes::HOTSPOTS.contains(&name) {
                out.warn(
                    "contract.hotspot-unknown",
                    Target::Node(name.into()),
                    format!("hotspot `{name}`: `{slot}` is no workshop slot"),
                );
            }
        }
        if name.starts_with("anchor_") && !n.primitives.is_empty() {
            out.warn(
                "contract.anchor-mesh",
                Target::Node(name.into()),
                format!("anchor `{name}` carries a mesh: anchors are empty points"),
            );
        }
        if name.starts_with("col_") {
            proxies += 1;
            if collision_shape(name).is_none() {
                out.error(
                    "contract.collision-shape",
                    Target::Node(name.into()),
                    format!(
                        "collision proxy `{name}`: the shape is one of col_box, col_cylinder, col_capsule (15B)"
                    ),
                );
            }
        }
    }
    if kind == Kind::Prop && proxies == 0 {
        out.warn(
            "contract.collision-none",
            Target::File,
            "a prop without a collision proxy `col_*` (Phase 12; fine for props that are only held or decor)",
        );
    }
    // Bounds: the kind's origin, the axes, the units.
    if let Some((lo, hi)) = m.near_bounds() {
        let size = hi - lo;
        if stands_on_ground(kind) {
            if lo.y < -GROUND_TOLERANCE_M {
                out.warn(
                    "bounds.below-ground",
                    Target::File,
                    format!(
                        "reaches {:.2} m below its origin: a {}'s origin is {} (15B)",
                        -lo.y,
                        kind.name(),
                        kind.origin()
                    ),
                );
            } else if lo.y > GROUND_TOLERANCE_M {
                out.warn(
                    "bounds.above-ground",
                    Target::File,
                    format!(
                        "floats {:.2} m above its origin: a {}'s origin is {} (15B)",
                        lo.y,
                        kind.name(),
                        kind.origin()
                    ),
                );
            }
        }
        if lies_on_its_back(kind, lo, hi) {
            out.warn(
                "bounds.on-its-back",
                Target::File,
                format!(
                    "{:.2} m high but {:.2} m deep: lying on its back? Export with \"+Y up\" (15A)",
                    size.y, size.z
                ),
            );
        }
        let longest = size.max_element();
        if longest > 100.0 {
            out.warn(
                "bounds.scale",
                Target::File,
                format!("{longest:.0} m long: exported in centimetres? Models are in metres (15B)"),
            );
        } else if longest < 0.005 {
            out.warn(
                "bounds.scale",
                Target::File,
                format!(
                    "{:.1} mm long: too small for metres (15B)",
                    longest * 1000.0
                ),
            );
        }
    }
    if bytes > kinds::WARN_FILE_BYTES {
        out.warn(
            "file.size",
            Target::File,
            format!(
                "{:.1} MB (models should stay below 2 MB)",
                bytes as f64 / 1e6
            ),
        );
    }
    out.list
}

/// [`check_entry`] and [`check_model`], errors first.
pub fn check(e: &Entry, m: &Model, bytes: usize) -> Vec<Finding> {
    let mut all = check_entry(e);
    all.extend(check_model(e, m, bytes));
    all.sort_by_key(|f| f.severity);
    all
}

/// Applies every fix of `findings` to the model (in order); the fixes that
/// changed it.
pub fn apply_fixes(m: &mut Model, e: &Entry, findings: &[Finding]) -> Vec<Fix> {
    let mut done = Vec::new();
    for f in findings {
        if let Some(fix) = &f.fix
            && !done.contains(fix)
            && fix.apply(m, e)
        {
            done.push(fix.clone());
        }
    }
    done
}

/// The node's name is one of the contract's (15B): an anchor, a hotspot,
/// a collision proxy or a level of detail.
pub fn contract_role(name: &str) -> Option<&'static str> {
    if name.starts_with("anchor_") {
        Some("anchor")
    } else if name.starts_with("hs_") {
        Some("hotspot")
    } else if name.starts_with("col_") {
        Some("collision proxy")
    } else if is_far_level(name) {
        Some("level of detail")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Parts;
    use crate::model::Node;
    use glam::Quat;

    fn entry(file: &str, kind: Kind) -> Entry {
        let mut e = crate::export::entry(file, kind, "test");
        e.generator = None;
        e
    }

    /// A small prop: a post of 0.1 × 1 × 0.1 m on the ground with a box
    /// proxy, stamped.
    pub(crate) fn post(file: &str) -> (Entry, Model) {
        let e = entry(file, Kind::Prop);
        let mut m = Model::default();
        let root = m.add(Node::new("post", None, Vec3::ZERO));
        let mut p = Parts::new();
        p.slot("metal").cuboid(
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.05, 0.5, 0.05),
            Quat::IDENTITY,
        );
        p.attach(&mut m, root);
        m.add(Node::new("col_box", Some(root), Vec3::new(0.0, 0.5, 0.0)));
        m.add(Node::new(
            "anchor_top",
            Some(root),
            Vec3::new(0.0, 1.0, 0.0),
        ));
        crate::export::stamp(&mut m, &e, serde_json::json!({}));
        (e, m)
    }

    fn codes(f: &[Finding]) -> Vec<&'static str> {
        f.iter().map(|f| f.code).collect()
    }

    #[test]
    fn a_clean_prop_has_no_findings() {
        let (e, m) = post("props/post-square-test.glb");
        let f = check(&e, &m, 1000);
        assert!(f.is_empty(), "{f:#?}");
    }

    #[test]
    fn blender_suffixes_and_near_slots_get_their_fix() {
        let (e, mut m) = post("props/post-square-test.glb");
        m.nodes[0].name = "post.001".into();
        m.materials[0].name = "Metal.002".into();
        let f = check(&e, &m, 1000);
        let suffix = f.iter().find(|f| f.code == "node.blender-suffix").unwrap();
        assert_eq!(
            suffix.fix,
            Some(Fix::RenameNode {
                from: "post.001".into(),
                to: "post".into()
            })
        );
        assert_eq!(suffix.severity, Severity::Warning);
        let slot = f.iter().find(|f| f.code == "material.not-slot").unwrap();
        assert!(slot.is_error());
        assert_eq!(
            slot.fix.as_ref().map(Fix::describe).as_deref(),
            Some("rename material `Metal.002` to `metal`")
        );
        // Applying the fixes leaves a clean model.
        let done = apply_fixes(&mut m, &e, &f);
        assert_eq!(done.len(), 2);
        assert!(check(&e, &m, 1000).is_empty());
        // The machine-readable form names the code, target and fix.
        let j: serde_json::Value = serde_json::from_str(&suffix.json()).unwrap();
        assert_eq!(j["code"], "node.blender-suffix");
        assert_eq!(j["severity"], "warning");
        assert_eq!(j["target"]["type"], "node");
        assert_eq!(j["target"]["name"], "post.001");
        assert_eq!(j["fix"]["fix"], "rename_node");
        assert_eq!(j["fix"]["to"], "post");
    }

    #[test]
    fn a_suffix_whose_name_is_taken_has_no_fix() {
        let (e, mut m) = post("props/post-square-test.glb");
        m.add(Node::new("post.001", Some(0), Vec3::ZERO));
        let f = check(&e, &m, 1000);
        let s = f.iter().find(|f| f.code == "node.blender-suffix").unwrap();
        assert!(s.fix.is_none() && s.message.contains("exists"), "{s:?}");
    }

    #[test]
    fn unused_materials_missing_extras_and_copyright_are_fixable() {
        let (e, mut m) = post("props/post-square-test.glb");
        m.materials.push(slots::material("frame"));
        m.extras = serde_json::Value::Null;
        m.copyright.clear();
        let f = check(&e, &m, 1000);
        assert_eq!(
            codes(&f),
            ["model.copyright", "extras.missing", "material.unused"]
        );
        apply_fixes(&mut m, &e, &f);
        assert!(check(&e, &m, 1000).is_empty());
        assert_eq!(m.ocw().unwrap()["id"], "props/post-square-test");
    }

    #[test]
    fn contract_names_are_checked() {
        let (e, mut m) = post("props/post-square-test.glb");
        m.nodes[1].name = "col_sphere".into();
        m.add(Node::new("hs_saddle", Some(0), Vec3::ZERO));
        let i = m.node("anchor_top").unwrap();
        m.nodes[i].primitives = m.nodes[0].primitives.clone();
        let f = check(&e, &m, 1000);
        let c = codes(&f);
        assert!(c.contains(&"contract.collision-shape"), "{c:?}");
        assert!(c.contains(&"contract.hotspot-misplaced"), "{c:?}");
        assert!(c.contains(&"contract.anchor-mesh"), "{c:?}");
        assert_eq!(collision_shape("col_cylinder_post"), Some("cylinder"));
        assert_eq!(collision_shape("col_boxy"), None);
        assert_eq!(contract_role("lod1"), Some("level of detail"));
        assert_eq!(contract_role("anchor_head"), Some("anchor"));
        // A prop without a proxy only warns.
        let (e, mut m) = post("props/post-square-test.glb");
        m.nodes.remove(1);
        let f = check(&e, &m, 1000);
        assert_eq!(codes(&f), ["contract.collision-none"]);
        assert_eq!(f[0].severity, Severity::Warning);
    }

    #[test]
    fn the_bounds_check_the_origin_axes_and_units() {
        // Sunk 30 cm into the ground.
        let (e, mut m) = post("props/post-square-test.glb");
        m.nodes[0].translation.y = -0.3;
        assert_eq!(codes(&check(&e, &m, 1000)), ["bounds.below-ground"]);
        m.nodes[0].translation.y = 0.3;
        assert_eq!(codes(&check(&e, &m, 1000)), ["bounds.above-ground"]);
        // In centimetres.
        let (e, mut m) = post("props/post-square-test.glb");
        m.nodes[0].scale = Vec3::splat(200.0);
        assert!(codes(&check(&e, &m, 1000)).contains(&"bounds.scale"));
        // A plant on its back: 1 m deep, 10 cm high.
        assert!(lies_on_its_back(
            Kind::Plant,
            Vec3::new(-0.1, 0.0, -0.5),
            Vec3::new(0.1, 0.1, 0.5)
        ));
        assert!(!lies_on_its_back(
            Kind::Plant,
            Vec3::new(-0.1, 0.0, -0.1),
            Vec3::new(0.1, 1.0, 0.1)
        ));
        assert!(!lies_on_its_back(
            Kind::Bike,
            Vec3::new(-0.1, 0.0, -0.5),
            Vec3::new(0.1, 0.1, 0.5)
        ));
    }

    #[test]
    fn entries_are_checked_with_hints() {
        let mut e = entry("wear/helmet-test-round.glb", Kind::Wearable);
        e.anchor = Some("hat".into());
        let f = check_entry(&e);
        assert_eq!(codes(&f), ["entry.anchor-unknown"]);
        e.anchor = Some("head".into());
        assert!(check_entry(&e).is_empty());
        e.license = "CC-BY-4.0".into();
        e.source = "a friend".into();
        assert_eq!(codes(&check_entry(&e)), ["entry.attribution-source"]);
        e.source = "https://example.org/models/helmet".into();
        assert!(check_entry(&e).is_empty());
        assert_eq!(strip_blender_suffix("wheel_front.001"), Some("wheel_front"));
        assert_eq!(strip_blender_suffix("v1.5"), None);
        assert_eq!(slot_for("Bar Tape"), Some("bar_tape"));
        assert_eq!(slot_for("gold"), None);
    }
}
