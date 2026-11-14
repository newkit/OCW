//! The asset manifest and validator (spec §2.5, §8.1 step 5, §13): every
//! 3D asset has an entry with `license`, `source` and `author`; the
//! validator rejects assets without one, licences outside the rule of
//! [`crate::licence`] (share-alike, non-commercial, no-derivatives,
//! unknown), physics values
//! in cosmetic assets, and models that miss the nodes the app animates.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::glb;
use crate::model::Model;
use crate::pose::{BikeRig, RiderRig, bike_nodes, nodes};
use crate::slots;

/// Keys that would make a cosmetic asset change physics (spec §2.5: only
/// a reference to a curated profile is allowed).
pub const PHYSICS_KEYS: [&str; 8] = [
    "mass",
    "mass_kg",
    "weight",
    "cda",
    "cda_m2",
    "crr",
    "efficiency",
    "drag",
];

/// Triangle budgets (spec §15: many riders on screen).
pub const MAX_TRIANGLES_BIKE: usize = 15_000;
pub const MAX_TRIANGLES_RIDER: usize = 8_000;
pub const MAX_TRIANGLES_LOD1: usize = 4_000;

/// What an asset is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A bike (one per physics profile family; `profile` names it).
    Bike,
    /// The rider (shared rig).
    Rider,
}

/// One manifest entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Entry {
    pub file: String,
    pub kind: Kind,
    /// Bikes: the id of the physics profile the model shows (a reference,
    /// never physics values).
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Deserialize)]
struct Raw {
    #[serde(default)]
    asset: Vec<toml::Table>,
}

/// A parsed manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub assets: Vec<Entry>,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, Vec<String>> {
        let raw: Raw = toml::from_str(text).map_err(|e| vec![format!("manifest: {e}")])?;
        let mut errors = Vec::new();
        let mut assets = Vec::new();
        for (i, t) in raw.asset.into_iter().enumerate() {
            let name = t
                .get("file")
                .and_then(|v| v.as_str())
                .map_or_else(|| format!("asset #{}", i + 1), str::to_owned);
            for k in t.keys() {
                if PHYSICS_KEYS.contains(&k.as_str()) {
                    errors.push(format!(
                        "{name}: `{k}` – cosmetic assets carry no physics, only a `profile` reference (spec §2.5)"
                    ));
                } else if !["file", "kind", "profile", "license", "source", "author"]
                    .contains(&k.as_str())
                {
                    errors.push(format!("{name}: unknown key `{k}`"));
                }
            }
            match toml::Value::Table(t).try_into::<Entry>() {
                Ok(e) => assets.push(e),
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        if errors.is_empty() {
            Ok(Self { assets })
        } else {
            Err(errors)
        }
    }
}

/// A checked asset.
#[derive(Debug, Clone)]
pub struct Checked {
    pub entry: Entry,
    pub model: Model,
    pub triangles: usize,
}

fn check_entry(e: &Entry) -> Vec<String> {
    let mut errors = Vec::new();
    let f = &e.file;
    for (key, value) in [
        ("license", &e.license),
        ("source", &e.source),
        ("author", &e.author),
    ] {
        if value.trim().is_empty() {
            errors.push(format!(
                "{f}: no `{key}` – every asset needs a licence entry (spec §13)"
            ));
        }
    }
    if !e.license.trim().is_empty()
        && let Err(why) = crate::licence::check(&e.license)
    {
        errors.push(format!("{f}: licence `{}`: {why}", e.license));
    }
    match (e.kind, &e.profile) {
        (Kind::Bike, None) => errors.push(format!("{f}: a bike needs its `profile`")),
        (Kind::Bike, Some(p)) if ocw_core::ProfileId::parse(p).is_none() => {
            errors.push(format!("{f}: unknown physics profile `{p}`"));
        }
        (Kind::Rider, Some(_)) => errors.push(format!("{f}: a rider has no `profile`")),
        _ => {}
    }
    errors
}

fn check_model(e: &Entry, m: &Model) -> Vec<String> {
    let f = &e.file;
    let mut errors = Vec::new();
    if m.copyright.trim().is_empty() {
        errors.push(format!(
            "{f}: no asset.copyright (the licence inside the file)"
        ));
    }
    for mat in &m.materials {
        if slots::get(&mat.name).is_none() {
            errors.push(format!("{f}: material `{}` is no paint slot", mat.name));
        }
    }
    let mut names = BTreeSet::new();
    for n in &m.nodes {
        if !n.name.is_empty() && !names.insert(n.name.as_str()) {
            errors.push(format!("{f}: node name `{}` twice", n.name));
        }
    }
    let tris = m.triangles();
    match e.kind {
        Kind::Bike => {
            for n in bike_nodes::REQUIRED.iter().chain(&bike_nodes::HOTSPOTS) {
                if m.node(n).is_none() {
                    errors.push(format!("{f}: missing node `{n}`"));
                }
            }
            if let Err(err) = BikeRig::from_model(m) {
                errors.push(format!("{f}: {err}"));
            }
            if tris > MAX_TRIANGLES_BIKE {
                errors.push(format!(
                    "{f}: {tris} triangles (budget {MAX_TRIANGLES_BIKE})"
                ));
            }
            if let Some(l) = m.node(bike_nodes::LOD1) {
                let far = m.triangles_below(Some(l));
                if far == 0 || far > MAX_TRIANGLES_LOD1 {
                    errors.push(format!(
                        "{f}: lod1 has {far} triangles (1…{MAX_TRIANGLES_LOD1})"
                    ));
                }
            }
        }
        Kind::Rider => {
            for n in nodes::REQUIRED {
                if m.node(n).is_none() {
                    errors.push(format!("{f}: missing node `{n}`"));
                }
            }
            if let Err(err) = RiderRig::from_model(m) {
                errors.push(format!("{f}: {err}"));
            }
            if tris > MAX_TRIANGLES_RIDER {
                errors.push(format!(
                    "{f}: {tris} triangles (budget {MAX_TRIANGLES_RIDER})"
                ));
            }
        }
    }
    errors
}

/// Validates a manifest against the asset files (name, bytes) next to it.
pub fn validate(manifest: &str, files: &[(String, Vec<u8>)]) -> Result<Vec<Checked>, Vec<String>> {
    let manifest = Manifest::parse(manifest)?;
    let mut errors = Vec::new();
    let mut checked = Vec::new();
    let mut listed = BTreeSet::new();
    for e in &manifest.assets {
        if !listed.insert(e.file.as_str()) {
            errors.push(format!("{}: listed twice", e.file));
        }
        errors.extend(check_entry(e));
        let Some((_, bytes)) = files.iter().find(|(n, _)| *n == e.file) else {
            errors.push(format!("{}: file missing", e.file));
            continue;
        };
        match glb::read(bytes) {
            Ok(m) => {
                let errs = check_model(e, &m);
                if errs.is_empty() {
                    checked.push(Checked {
                        entry: e.clone(),
                        triangles: m.triangles(),
                        model: m,
                    });
                }
                errors.extend(errs);
            }
            Err(err) => errors.push(format!("{}: {err}", e.file)),
        }
    }
    for (name, _) in files {
        let is_model = name.ends_with(".glb") || name.ends_with(".gltf");
        if is_model && !listed.contains(name.as_str()) {
            errors.push(format!(
                "{name}: not in the manifest – an asset without a licence entry (spec §13)"
            ));
        }
    }
    if errors.is_empty() {
        Ok(checked)
    } else {
        Err(errors)
    }
}

/// Validates the manifest `dir/manifest.toml` and the models in `dir`.
pub fn validate_dir(dir: &std::path::Path) -> Result<Vec<Checked>, Vec<String>> {
    let text = std::fs::read_to_string(dir.join(MANIFEST))
        .map_err(|e| vec![format!("{}: {e}", dir.join(MANIFEST).display())])?;
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| vec![format!("{}: {e}", dir.display())])?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".glb") || name.ends_with(".gltf") {
            let bytes = std::fs::read(entry.path()).map_err(|e| vec![format!("{name}: {e}")])?;
            files.push((name, bytes));
        }
    }
    files.sort();
    validate(&text, &files)
}

/// File name of the manifest in an asset directory.
pub const MANIFEST: &str = "manifest.toml";

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(file: &str, extra: &str) -> String {
        format!("[[asset]]\nfile = \"{file}\"\nkind = \"bike\"\nprofile = \"road\"\n{extra}\n")
    }

    const OK: &str =
        "license = \"MIT OR Apache-2.0\"\nsource = \"generated\"\nauthor = \"OCW contributors\"";

    fn files() -> Vec<(String, Vec<u8>)> {
        let r = crate::rider::build(crate::rider::Detail::High);
        let low = crate::rider::build(crate::rider::Detail::Low);
        let mut m = crate::bikes::build(ocw_core::ProfileId::Road, &r, &low);
        m.copyright = "test".into();
        vec![("b.glb".into(), glb::write(&m))]
    }

    #[test]
    fn a_complete_entry_passes() {
        let ok = validate(&entry("b.glb", OK), &files()).unwrap();
        assert_eq!(ok.len(), 1);
        assert!(ok[0].triangles > 1000);
    }

    #[test]
    fn assets_without_a_licence_entry_are_rejected() {
        // No entry at all for a model file.
        let errs = validate("", &files()).unwrap_err();
        assert!(errs[0].contains("not in the manifest"), "{errs:?}");
        // An entry without licence / source / author.
        let errs = validate(&entry("b.glb", ""), &files()).unwrap_err();
        assert_eq!(errs.len(), 3, "{errs:?}");
        assert!(errs.iter().all(|e| e.contains("licence entry")));
        // Unknown or share-alike licences.
        let gpl = OK.replace("MIT OR Apache-2.0", "CC-BY-NC-4.0");
        let errs = validate(&entry("b.glb", &gpl), &files()).unwrap_err();
        assert!(errs[0].contains("non-commercial"), "{errs:?}");
        // A listed file that is not there.
        let errs = validate(&entry("gone.glb", OK), &files()).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("gone.glb: file missing")),
            "{errs:?}"
        );
    }

    /// Cosmetic parts never change physics (spec §2.5): the manifest may
    /// only reference a curated profile.
    #[test]
    fn physics_values_in_a_cosmetic_asset_are_rejected() {
        let errs = validate(
            &entry("b.glb", &format!("{OK}\nmass_kg = 6.0\ncda = 0.2")),
            &files(),
        )
        .unwrap_err();
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert!(errs.iter().all(|e| e.contains("no physics")));
        let unknown = entry("b.glb", OK).replace("\"road\"", "\"rocket\"");
        let errs = validate(&unknown, &files()).unwrap_err();
        assert!(errs[0].contains("unknown physics profile"), "{errs:?}");
    }

    #[test]
    fn models_missing_animated_nodes_are_rejected() {
        let mut m = glb::read(&files()[0].1).unwrap();
        let i = m.node("crank").unwrap();
        m.nodes[i].name = "spinny".into();
        m.materials[0].name = "gold".into();
        let errs = validate(&entry("b.glb", OK), &[("b.glb".into(), glb::write(&m))]).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("missing node `crank`")),
            "{errs:?}"
        );
        assert!(errs.iter().any(|e| e.contains("no paint slot")), "{errs:?}");
        // A rider file declared as a bike lacks the bike's nodes.
        let rider = glb::write(&crate::rider::build(crate::rider::Detail::High));
        let errs = validate(&entry("b.glb", OK), &[("b.glb".into(), rider)]).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("missing node `wheel_front`"))
        );
        assert!(errs.iter().any(|e| e.contains("copyright")));
    }

    /// The workshop needs a hotspot anchor per part slot (spec §14 Phase
    /// 4, 4B): a bike without one is rejected.
    #[test]
    fn bikes_missing_a_hotspot_anchor_are_rejected() {
        let mut m = glb::read(&files()[0].1).unwrap();
        let i = m.node("hs_lights").unwrap();
        m.nodes[i].name = "lamp".into();
        let errs = validate(&entry("b.glb", OK), &[("b.glb".into(), glb::write(&m))]).unwrap_err();
        assert_eq!(errs, vec!["b.glb: missing node `hs_lights`".to_string()]);
    }
}
