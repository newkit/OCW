//! `model import` and `model export` (spec §14 Phase 15, M2; 15D): a glTF
//! from Blender or any tool becomes the canonical `.glb` of the model
//! folder with its manifest entry, and a model goes out as an editable
//! copy (`.glb`, or `.gltf` + `.bin` with readable JSON).
//!
//! The import normalises what Blender's exporter leaves:
//! - Blender's duplicate suffixes (`wheel_front.001`) are stripped where
//!   the plain name is free;
//! - material names close to a paint slot (`Frame.001`, `Bar Tape`) become
//!   the slot; materials of the same slot are merged;
//! - the primitives of a node are merged per material, unused materials
//!   dropped, missing normals computed;
//! - `asset.extras.ocw` is stamped from the entry (other keys kept),
//!   the copyright and generator set.
//!
//! Then the contract is checked ([`crate::check`]): a model lying on its
//! back (exported without "+Y up") or with any other error is refused
//! with the findings and their fixes; warnings are passed on.

use crate::check::{self, Finding, Severity};
use crate::kinds::{self, Kind};
use crate::manifest::{Entry, Manifest};
use crate::model::{Model, Primitive};

/// What the import writes into the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    pub kind: Kind,
    /// The id: `bike-road-test` or `bikes/bike-road-test` (the kind's
    /// folder is added when missing).
    pub id: String,
    pub license: String,
    pub source: String,
    pub author: String,
    pub profile: Option<String>,
    pub species: Option<String>,
    pub variant: Option<String>,
    pub anchor: Option<String>,
}

impl ImportOptions {
    /// Own work (MIT OR Apache-2.0, the OCW contributors).
    pub fn own(kind: Kind, id: &str) -> Self {
        Self {
            kind,
            id: id.into(),
            license: crate::export::LICENSE.into(),
            source: "own work, imported with `ocw-assets model import` (spec §14 Phase 15, 15D)"
                .into(),
            author: crate::export::AUTHOR.into(),
            profile: None,
            species: None,
            variant: None,
            anchor: None,
        }
    }

    /// The manifest entry.
    pub fn entry(&self) -> Entry {
        let id = self.id.trim_end_matches(".glb");
        let file = if id.starts_with(&format!("{}/", self.kind.dir())) {
            format!("{id}.glb")
        } else {
            format!("{}/{id}.glb", self.kind.dir())
        };
        Entry {
            file,
            kind: self.kind,
            profile: self.profile.clone(),
            species: self.species.clone(),
            variant: self.variant.clone(),
            anchor: self.anchor.clone(),
            generator: None,
            license: self.license.clone(),
            source: self.source.clone(),
            author: self.author.clone(),
        }
    }
}

/// The result of an import.
#[derive(Debug, Clone)]
pub struct Imported {
    pub entry: Entry,
    pub model: Model,
    /// The canonical file.
    pub bytes: Vec<u8>,
    /// What the normalisation changed.
    pub notes: Vec<String>,
    /// The validator's warnings.
    pub warnings: Vec<Finding>,
}

/// Normalises a model as Blender exports it (see the module); the notes
/// say what changed.
pub fn normalise(m: &mut Model) -> Vec<String> {
    let mut notes = Vec::new();
    // Blender's `.001` suffixes, where the plain name is free and only one
    // node wants it.
    let wanted: Vec<Option<String>> = m
        .nodes
        .iter()
        .map(|n| check::strip_blender_suffix(&n.name).map(str::to_owned))
        .collect();
    for i in 0..m.nodes.len() {
        let Some(base) = &wanted[i] else { continue };
        let rivals = wanted.iter().filter(|w| w.as_ref() == Some(base)).count();
        if rivals == 1 && m.node(base).is_none() {
            notes.push(format!("node `{}` renamed to `{base}`", m.nodes[i].name));
            m.nodes[i].name.clone_from(base);
        }
    }
    // Materials to their paint slots, merged per slot.
    let mut remap: Vec<usize> = Vec::with_capacity(m.materials.len());
    let mut kept: Vec<crate::model::Material> = Vec::new();
    for mat in &m.materials {
        let name = if crate::slots::get(&mat.name).is_some() {
            mat.name.clone()
        } else if let Some(slot) = check::slot_for(&mat.name) {
            notes.push(format!("material `{}` renamed to `{slot}`", mat.name));
            slot.to_owned()
        } else {
            mat.name.clone()
        };
        match kept.iter().position(|k| k.name == name) {
            Some(j) => {
                notes.push(format!("material `{}` merged into `{name}`", mat.name));
                remap.push(j);
            }
            None => {
                let mut k = mat.clone();
                k.name = name;
                kept.push(k);
                remap.push(kept.len() - 1);
            }
        }
    }
    m.materials = kept;
    // Primitives merged per material within a node.
    for n in &mut m.nodes {
        for p in &mut n.primitives {
            p.material = remap.get(p.material).copied().unwrap_or(p.material);
        }
        let before = n.primitives.len();
        n.primitives = merge_by_material(std::mem::take(&mut n.primitives));
        if n.primitives.len() < before {
            notes.push(format!(
                "node `{}`: {before} primitives merged into {}",
                n.name,
                n.primitives.len()
            ));
        }
    }
    // Unused materials dropped.
    let mut i = 0;
    while i < m.materials.len() {
        let used = m
            .nodes
            .iter()
            .any(|n| n.primitives.iter().any(|p| p.material == i));
        if used {
            i += 1;
            continue;
        }
        notes.push(format!("unused material `{}` dropped", m.materials[i].name));
        m.materials.remove(i);
        for n in &mut m.nodes {
            for p in &mut n.primitives {
                if p.material > i {
                    p.material -= 1;
                }
            }
        }
    }
    notes
}

/// Merges primitives of the same material (in first-seen order); vertex
/// colours and UVs missing on some of them are filled (white, 0).
fn merge_by_material(prims: Vec<Primitive>) -> Vec<Primitive> {
    let mut out: Vec<Primitive> = Vec::new();
    let colours = |mats: &[Primitive], mat: usize| {
        mats.iter()
            .any(|p| p.material == mat && !p.colors.is_empty())
    };
    let uvs = |mats: &[Primitive], mat: usize| {
        mats.iter().any(|p| p.material == mat && !p.uvs.is_empty())
    };
    let want: Vec<(usize, bool, bool)> = {
        let mut seen = Vec::new();
        for p in &prims {
            if !seen.iter().any(|(m, _, _)| *m == p.material) {
                seen.push((
                    p.material,
                    colours(&prims, p.material),
                    uvs(&prims, p.material),
                ));
            }
        }
        seen
    };
    for (mat, has_c, has_uv) in want {
        let mut q = Primitive {
            material: mat,
            ..Default::default()
        };
        for p in prims.iter().filter(|p| p.material == mat) {
            let base = q.positions.len() as u32;
            q.positions.extend_from_slice(&p.positions);
            q.normals.extend_from_slice(&p.normals);
            q.indices.extend(p.indices.iter().map(|i| i + base));
            if has_c {
                if p.colors.is_empty() {
                    q.colors
                        .extend(std::iter::repeat_n([1.0; 4], p.positions.len()));
                } else {
                    q.colors.extend_from_slice(&p.colors);
                }
            }
            if has_uv {
                if p.uvs.is_empty() {
                    q.uvs
                        .extend(std::iter::repeat_n([0.0; 2], p.positions.len()));
                } else {
                    q.uvs.extend_from_slice(&p.uvs);
                }
            }
        }
        out.push(q);
    }
    out
}

/// Imports a read model (see the module): the canonical file, or the
/// errors (each with its fix where clear).
pub fn import(
    mut m: Model,
    read_notes: Vec<String>,
    opts: &ImportOptions,
) -> Result<Imported, Vec<String>> {
    let entry = opts.entry();
    let mut notes = read_notes;
    notes.extend(normalise(&mut m));
    // The contract's metadata.
    check::Fix::StampExtras.apply(&mut m, &entry);
    m.generator = crate::GENERATOR.into();
    if m.copyright.trim().is_empty() {
        m.copyright = if entry.license == crate::export::LICENSE {
            crate::COPYRIGHT.into()
        } else {
            format!("{}; licence {}", entry.author, entry.license)
        };
    }
    // The axes: a model lying on its back is refused (15D).
    let mut errors = Vec::new();
    if let Some((lo, hi)) = m.near_bounds()
        && check::lies_on_its_back(entry.kind, lo, hi)
    {
        errors.push(format!(
            "{}: the model lies on its back ({:.2} m high, {:.2} m deep): export it from Blender with \"+Y up\" (15A), or turn it −90° about X",
            entry.file,
            hi.y - lo.y,
            hi.z - lo.z
        ));
    }
    let bytes = crate::glb::write(&m);
    // Check the canonical file as it will be read.
    let canonical = crate::glb::read(&bytes).map_err(|e| vec![format!("{}: {e}", entry.file)])?;
    let findings = check::check(&entry, &canonical, bytes.len());
    for f in findings.iter().filter(|f| f.severity == Severity::Error) {
        let hint = f
            .fix
            .as_ref()
            .map(|x| format!(" (fix: {})", x.describe()))
            .unwrap_or_default();
        errors.push(format!("{}{hint}", f.line()));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Imported {
        entry,
        model: canonical,
        bytes,
        notes,
        warnings: findings.into_iter().filter(|f| !f.is_error()).collect(),
    })
}

/// The manifest `old` with `e` added, or replacing the entry of the same
/// file (sorted, as the exporters write it).
pub fn upsert_manifest(old: &str, e: &Entry) -> Result<String, Vec<String>> {
    let mut all = if old.trim().is_empty() {
        Vec::new()
    } else {
        Manifest::parse(old)?.assets
    };
    all.retain(|x| x.file != e.file);
    all.push(e.clone());
    Ok(crate::export::manifest_text(&all))
}

/// The file name of an editable copy of `id` (`bikes/bike-tt` →
/// `bike-tt.glb` / `bike-tt.gltf`).
pub fn export_name(id: &str, gltf: bool) -> String {
    let stem = kinds::id_of(id).rsplit('/').next().unwrap_or(id);
    format!("{stem}.{}", if gltf { "gltf" } else { "glb" })
}

/// The editable copy of a model: (file name, bytes) – one `.glb`, or a
/// `.gltf` and its `.bin` (spec §14 Phase 15, 15D `model export`).
pub fn export_files(id: &str, m: &Model, gltf: bool) -> Vec<(String, Vec<u8>)> {
    if !gltf {
        return vec![(export_name(id, false), crate::glb::write(m))];
    }
    let name = export_name(id, true);
    let bin = format!("{}.bin", name.trim_end_matches(".gltf"));
    let (text, data) = crate::glb::write_gltf(m, &bin);
    vec![(name, text.into_bytes()), (bin, data)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glb;
    use crate::mesh::Parts;
    use crate::model::Node;
    use glam::{Quat, Vec3};

    /// A post shaped like a Blender export: suffixes, a material in Blender
    /// case, an unused material, two primitives of one material, no
    /// normals on one primitive, no extras.
    fn blender_post() -> Model {
        let mut m = Model::default();
        let root = m.add(Node::new("post.001", None, Vec3::ZERO));
        let mut p = Parts::new();
        p.slot("metal").cuboid(
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.05, 0.5, 0.05),
            Quat::IDENTITY,
        );
        p.attach(&mut m, root);
        let mut q = Parts::new();
        q.slot("metal")
            .cuboid(Vec3::new(0.0, 1.05, 0.0), Vec3::splat(0.06), Quat::IDENTITY);
        q.attach(&mut m, root);
        m.materials[0].name = "Metal.001".into();
        m.materials.push(crate::slots::material("frame"));
        m.add(Node::new(
            "col_box.001",
            Some(root),
            Vec3::new(0.0, 0.5, 0.0),
        ));
        m
    }

    #[test]
    fn a_blender_export_is_normalised() {
        let opts = ImportOptions::own(Kind::Prop, "post-square-test");
        let m = blender_post();
        assert_eq!(m.nodes[0].primitives.len(), 2);
        let out = import(m, vec![], &opts).unwrap();
        assert_eq!(out.entry.file, "props/post-square-test.glb");
        let m = &out.model;
        assert_eq!(m.nodes[0].name, "post");
        assert_eq!(m.nodes[1].name, "col_box");
        assert_eq!(m.materials.len(), 1);
        assert_eq!(m.materials[0].name, "metal");
        assert_eq!(m.nodes[0].primitives.len(), 1);
        assert_eq!(m.ocw().unwrap()["id"], "props/post-square-test");
        assert_eq!(m.ocw().unwrap()["kind"], "prop");
        assert_eq!(m.copyright, crate::COPYRIGHT);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert!(out.notes.iter().any(|n| n.contains("renamed to `post`")));
        assert!(out.notes.iter().any(|n| n.contains("dropped")));
        // Importing the canonical file again changes nothing.
        let again = import(glb::read(&out.bytes).unwrap(), vec![], &opts).unwrap();
        assert_eq!(again.bytes, out.bytes);
        assert!(again.notes.is_empty(), "{:?}", again.notes);
    }

    #[test]
    fn missing_normals_are_computed() {
        let m = blender_post();
        let mut doc = glb::write(&m);
        // Drop NORMAL from the first primitive's attributes in the JSON.
        let len = u32::from_le_bytes(doc[12..16].try_into().unwrap()) as usize;
        let text = String::from_utf8(doc[20..20 + len].to_vec()).unwrap();
        let mut j: serde_json::Value = serde_json::from_str(&text).unwrap();
        j["meshes"][0]["primitives"][0]["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("NORMAL");
        let mut t = serde_json::to_vec(&j).unwrap();
        while t.len() < len {
            t.push(b' ');
        }
        assert_eq!(t.len(), len, "the shortened JSON keeps its chunk length");
        doc[20..20 + len].copy_from_slice(&t);
        assert!(glb::read(&doc).is_err());
        let (read, notes) = glb::read_opts(&doc, true, &|_| None).unwrap();
        assert_eq!(notes, vec!["node post.001: normals computed".to_string()]);
        let n = &read.nodes[0].primitives[0].normals;
        assert!(
            n.iter()
                .all(|v| (Vec3::from(*v).length() - 1.0).abs() < 1e-4)
        );
        let out = import(
            read,
            notes,
            &ImportOptions::own(Kind::Prop, "post-square-test"),
        )
        .unwrap();
        assert!(out.notes[0].contains("normals computed"));
    }

    #[test]
    fn a_model_on_its_back_and_contract_errors_are_refused() {
        // A plant lying on its back: 1 m deep, 10 cm high.
        let mut m = Model::default();
        let root = m.add(Node::new("plant", None, Vec3::ZERO));
        let mut p = Parts::new();
        p.slot("leaf").cuboid(
            Vec3::new(0.0, 0.05, 0.0),
            Vec3::new(0.1, 0.05, 0.5),
            Quat::IDENTITY,
        );
        p.attach(&mut m, root);
        let mut opts = ImportOptions::own(Kind::Plant, "tree-test-round-a");
        opts.species = Some("Beech".into());
        let errs = import(m, vec![], &opts).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("lies on its back")),
            "{errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.contains("missing node `lod1`")),
            "{errs:?}"
        );
        // A material that is no slot: refused, no fix to offer.
        let mut m = blender_post();
        m.materials[0].name = "gold".into();
        let errs = import(
            m,
            vec![],
            &ImportOptions::own(Kind::Prop, "post-square-test"),
        )
        .unwrap_err();
        assert!(errs.iter().any(|e| e.contains("no paint slot")), "{errs:?}");
    }

    #[test]
    fn the_manifest_gets_the_entry_and_exports_round_trip() {
        let opts = ImportOptions::own(Kind::Prop, "props/post-square-test");
        let out = import(blender_post(), vec![], &opts).unwrap();
        let text = upsert_manifest("", &out.entry).unwrap();
        let again = upsert_manifest(&text, &out.entry).unwrap();
        assert_eq!(text, again);
        let man = Manifest::parse(&text).unwrap();
        assert_eq!(man.assets, vec![out.entry.clone()]);
        // The whole folder validates.
        let checked =
            crate::manifest::validate(&text, &[(out.entry.file.clone(), out.bytes.clone())])
                .unwrap();
        assert_eq!(checked.len(), 1);
        // The editable copies read back as the same model.
        let files = export_files("props/post-square-test", &out.model, true);
        assert_eq!(files[0].0, "post-square-test.gltf");
        assert_eq!(files[1].0, "post-square-test.bin");
        let bin = files[1].1.clone();
        let back = glb::read_with(&files[0].1, &|uri| {
            (uri == "post-square-test.bin").then(|| bin.clone())
        })
        .unwrap();
        assert!(crate::diff::diff(&out.model, &back).is_empty());
        let glb_copy = export_files("props/post-square-test", &out.model, false);
        assert_eq!(glb_copy[0].1, out.bytes);
        // The text form is readable JSON with the extras kept.
        let j: serde_json::Value = serde_json::from_slice(&files[0].1).unwrap();
        assert_eq!(j["asset"]["extras"]["ocw"]["kind"], "prop");
    }
}
