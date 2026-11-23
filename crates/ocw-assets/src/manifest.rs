//! The asset manifest and validator (spec §2.5, §8.1 step 5, §13): every
//! 3D asset has an entry with `license`, `source` and `author`; the
//! validator rejects assets without one, licences outside the rule of
//! [`crate::licence`] (share-alike, non-commercial, no-derivatives,
//! unknown), physics values
//! in cosmetic assets, and models that miss the nodes the app animates.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::check::{self, Finding, Severity, Target};
use crate::glb;
use crate::model::Model;

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

/// Triangle budgets (spec §15: many riders on screen; per kind in
/// [`crate::kinds::Kind::budget`]).
pub const MAX_TRIANGLES_BIKE: usize = 15_000;
pub const MAX_TRIANGLES_RIDER: usize = 8_000;
pub const MAX_TRIANGLES_LOD1: usize = 4_000;

pub use crate::kinds::Kind;

/// The keys an entry may have.
pub const KEYS: [&str; 10] = [
    "file",
    "kind",
    "profile",
    "species",
    "variant",
    "anchor",
    "generator",
    "license",
    "source",
    "author",
];

/// One manifest entry (spec §14 Phase 15, 15C).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Entry {
    /// The path in the model folder (`bikes/bike-tt.glb`); the id is it
    /// without `.glb` ([`crate::kinds::id_of`]).
    pub file: String,
    pub kind: Kind,
    /// Bikes: the id of the physics profile the model shows (a reference,
    /// never physics values).
    #[serde(default)]
    pub profile: Option<String>,
    /// Plants: the species (`ocw_world::flora::Species::name`).
    #[serde(default)]
    pub species: Option<String>,
    /// A shape variant's letter (plants: `a`, `b`, `c`, …).
    #[serde(default)]
    pub variant: Option<String>,
    /// Wearables: the rider node it is worn at (`head`, …).
    #[serde(default)]
    pub anchor: Option<String>,
    /// The code the file was exported from, with its parameters (until
    /// the generators are removed, Q6).
    #[serde(default)]
    pub generator: Option<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub author: String,
}

impl Entry {
    /// The model's id: its path without `.glb`.
    pub fn id(&self) -> &str {
        crate::kinds::id_of(&self.file)
    }
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
                } else if !KEYS.contains(&k.as_str()) {
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
    /// The file's size.
    pub bytes: usize,
    /// The extended validator's findings for this file and its entry
    /// (spec §14 Phase 15, M2; [`crate::check`]).
    pub findings: Vec<Finding>,
}

impl Checked {
    /// Warnings that do not fail the check (files above 2 MB, unused
    /// materials, Blender suffixes, bounds, …).
    pub fn warnings(&self) -> Vec<String> {
        self.findings
            .iter()
            .filter(|f| !f.is_error())
            .map(Finding::line)
            .collect()
    }
}

/// Everything the validator found in a model folder.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Every listed model that could be read.
    pub checked: Vec<Checked>,
    /// All findings, errors and warnings, in manifest order (the files
    /// missing from the manifest last).
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.is_error())
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| !f.is_error())
    }
}

fn file_error(file: &str, code: &'static str, message: String) -> Finding {
    Finding {
        severity: Severity::Error,
        code,
        file: file.to_owned(),
        target: Target::File,
        message,
        fix: None,
    }
}

/// Validates a manifest against the asset files (name, bytes) next to it
/// and returns every finding; `Err` only when the manifest cannot be
/// read.
pub fn report(manifest: &str, files: &[(String, Vec<u8>)]) -> Result<Report, Vec<String>> {
    let manifest = Manifest::parse(manifest)?;
    let mut out = Report::default();
    let mut listed = BTreeSet::new();
    for e in &manifest.assets {
        if !listed.insert(e.file.as_str()) {
            out.findings.push(file_error(
                &e.file,
                "entry.duplicate",
                "listed twice".into(),
            ));
        }
        let Some((_, bytes)) = files.iter().find(|(n, _)| *n == e.file) else {
            out.findings.extend(check::check_entry(e));
            out.findings
                .push(file_error(&e.file, "file.missing", "file missing".into()));
            continue;
        };
        match glb::read(bytes) {
            Ok(m) => {
                let findings = check::check(e, &m, bytes.len());
                out.findings.extend(findings.iter().cloned());
                out.checked.push(Checked {
                    entry: e.clone(),
                    triangles: m.triangles(),
                    model: m,
                    bytes: bytes.len(),
                    findings,
                });
            }
            Err(err) => {
                out.findings.extend(check::check_entry(e));
                out.findings
                    .push(file_error(&e.file, "file.unreadable", err.to_string()));
            }
        }
    }
    for (name, _) in files {
        let is_model = name.ends_with(".glb") || name.ends_with(".gltf");
        if is_model && !listed.contains(name.as_str()) {
            out.findings.push(file_error(
                name,
                "entry.unlisted",
                "not in the manifest – an asset without a licence entry (spec §13)".into(),
            ));
        }
    }
    Ok(out)
}

/// Validates a manifest against the asset files (name, bytes) next to it:
/// the checked assets, or every error (`FILE: message`).
pub fn validate(manifest: &str, files: &[(String, Vec<u8>)]) -> Result<Vec<Checked>, Vec<String>> {
    let r = report(manifest, files)?;
    let errors: Vec<String> = r.errors().map(Finding::line).collect();
    if errors.is_empty() {
        Ok(r.checked)
    } else {
        Err(errors)
    }
}

/// Reads every model file below `dir` as (path relative to `dir` with
/// `/`, bytes), sorted. `sources/` (the Blender sources) is skipped.
pub fn read_dir_models(dir: &std::path::Path) -> Result<Vec<(String, Vec<u8>)>, Vec<String>> {
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), Vec<String>> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| vec![format!("{}: {e}", dir.display())])?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if entry.file_name() != "sources" {
                    walk(root, &path, out)?;
                }
                continue;
            }
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if name.ends_with(".glb") || name.ends_with(".gltf") {
                let bytes = std::fs::read(&path).map_err(|e| vec![format!("{name}: {e}")])?;
                out.push((name, bytes));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    files.sort();
    Ok(files)
}

/// Validates the manifest `dir/manifest.toml` and the models below `dir`.
pub fn validate_dir(dir: &std::path::Path) -> Result<Vec<Checked>, Vec<String>> {
    let text = std::fs::read_to_string(dir.join(MANIFEST))
        .map_err(|e| vec![format!("{}: {e}", dir.join(MANIFEST).display())])?;
    let files = read_dir_models(dir)?;
    validate(&text, &files)
}

/// [`report`] for the folder `dir` (its manifest and models).
pub fn report_dir(dir: &std::path::Path) -> Result<Report, Vec<String>> {
    let text = std::fs::read_to_string(dir.join(MANIFEST))
        .map_err(|e| vec![format!("{}: {e}", dir.join(MANIFEST).display())])?;
    let files = read_dir_models(dir)?;
    report(&text, &files)
}

/// File name of the manifest in an asset directory.
pub const MANIFEST: &str = "manifest.toml";

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "bikes/bike-test-road.glb";

    fn entry(file: &str, extra: &str) -> String {
        format!("[[asset]]\nfile = \"{file}\"\nkind = \"bike\"\nprofile = \"road\"\n{extra}\n")
    }

    fn stamped(mut m: Model) -> Model {
        let e = Manifest::parse(&entry(B, OK)).unwrap().assets.remove(0);
        crate::export::stamp(&mut m, &e, serde_json::json!({}));
        m.copyright = "test".into();
        m
    }

    const OK: &str =
        "license = \"MIT OR Apache-2.0\"\nsource = \"generated\"\nauthor = \"OCW contributors\"";

    fn files() -> Vec<(String, Vec<u8>)> {
        let r = crate::rider::build(crate::rider::Detail::High);
        let low = crate::rider::build(crate::rider::Detail::Low);
        let m = stamped(crate::bikes::build(ocw_core::ProfileId::Road, &r, &low));
        vec![(B.into(), glb::write(&m))]
    }

    #[test]
    fn a_complete_entry_passes() {
        let ok = validate(&entry(B, OK), &files()).unwrap();
        assert_eq!(ok.len(), 1);
        assert!(ok[0].triangles > 1000);
    }

    #[test]
    fn assets_without_a_licence_entry_are_rejected() {
        // No entry at all for a model file.
        let errs = validate("", &files()).unwrap_err();
        assert!(errs[0].contains("not in the manifest"), "{errs:?}");
        // An entry without licence / source / author.
        let errs = validate(&entry(B, ""), &files()).unwrap_err();
        assert_eq!(errs.len(), 3, "{errs:?}");
        assert!(errs.iter().all(|e| e.contains("licence entry")));
        // Unknown or share-alike licences.
        let gpl = OK.replace("MIT OR Apache-2.0", "CC-BY-NC-4.0");
        let errs = validate(&entry(B, &gpl), &files()).unwrap_err();
        assert!(errs[0].contains("non-commercial"), "{errs:?}");
        // A listed file that is not there.
        let errs = validate(&entry("bikes/bike-gone-road.glb", OK), &files()).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("bike-gone-road.glb: file missing")),
            "{errs:?}"
        );
    }

    /// Cosmetic parts never change physics (spec §2.5): the manifest may
    /// only reference a curated profile.
    #[test]
    fn physics_values_in_a_cosmetic_asset_are_rejected() {
        let errs = validate(
            &entry(B, &format!("{OK}\nmass_kg = 6.0\ncda = 0.2")),
            &files(),
        )
        .unwrap_err();
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert!(errs.iter().all(|e| e.contains("no physics")));
        let unknown = entry(B, OK).replace("\"road\"", "\"rocket\"");
        let errs = validate(&unknown, &files()).unwrap_err();
        assert!(errs[0].contains("unknown physics profile"), "{errs:?}");
    }

    #[test]
    fn models_missing_animated_nodes_are_rejected() {
        let mut m = glb::read(&files()[0].1).unwrap();
        let i = m.node("crank").unwrap();
        m.nodes[i].name = "spinny".into();
        m.materials[0].name = "gold".into();
        let errs = validate(&entry(B, OK), &[(B.into(), glb::write(&m))]).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("missing node `crank`")),
            "{errs:?}"
        );
        assert!(errs.iter().any(|e| e.contains("no paint slot")), "{errs:?}");
        // A rider file declared as a bike lacks the bike's nodes.
        let rider = glb::write(&crate::rider::build(crate::rider::Detail::High));
        let errs = validate(&entry(B, OK), &[(B.into(), rider)]).unwrap_err();
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
        let errs = validate(&entry(B, OK), &[(B.into(), glb::write(&m))]).unwrap_err();
        assert_eq!(errs, vec![format!("{B}: missing node `hs_lights`")]);
    }

    /// The contract of 15B / 15H: names, the asset's `extras.ocw`, kinds'
    /// own keys and budgets.
    #[test]
    fn the_model_contract_is_checked() {
        // A file outside its kind's folder, or named against the pattern.
        let errs = validate(
            &entry("bike-road.glb", OK),
            &[("bike-road.glb".into(), files()[0].1.clone())],
        )
        .unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("lives in bikes/")),
            "{errs:?}"
        );
        // No extras.ocw, or the wrong id.
        let mut m = glb::read(&files()[0].1).unwrap();
        m.extras = serde_json::Value::Null;
        let errs = validate(&entry(B, OK), &[(B.into(), glb::write(&m))]).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("no asset.extras.ocw")),
            "{errs:?}"
        );
        m.extras = serde_json::json!({"ocw": {"schema": 1, "kind": "bike", "id": "bikes/other", "mass_kg": 7}});
        let errs = validate(&entry(B, OK), &[(B.into(), glb::write(&m))]).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("extras.ocw.id")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("no physics")), "{errs:?}");
        // A plant needs its species and a lod1; a wearable its anchor.
        let plant = "[[asset]]\nfile = \"flora/tree-test-round-a.glb\"\nkind = \"plant\"\n";
        let errs = validate(&format!("{plant}{OK}\n"), &[]).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("needs its `species`")),
            "{errs:?}"
        );
        let wear = "[[asset]]\nfile = \"wear/helmet-test-round.glb\"\nkind = \"wearable\"\nprofile = \"road\"\n";
        let errs = validate(&format!("{wear}{OK}\n"), &[]).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("needs its `anchor`")),
            "{errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.contains("has no `profile`")),
            "{errs:?}"
        );
    }
}
