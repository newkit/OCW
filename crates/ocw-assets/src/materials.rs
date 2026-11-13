//! The material library (spec §4.6, §8.1, §13, Phase 8 T1): a small
//! curated set of tiling PBR materials for the ground, the roads and (later,
//! T4) the houses.
//!
//! **Shipping (spec §4.6 "Material library"):** the textures are not in
//! git. `assets/materials.toml` (in git) pins every source file – URL (or
//! member of a pinned archive) and SHA-256 – with its licence, author and
//! source page. `scripts/fetch-materials.sh` (`ocw-assets materials
//! fetch`) downloads the sources once into a cache, checks every hash,
//! converts them to KTX2 ([`crate::convert`]) and writes the library into
//! the git-ignored `assets/materials/` together with a copy of the
//! manifest and `CREDITS.md`. Release packages and the test deployment
//! carry that folder; the app finds it beside the binary
//! ([`library_candidates`]) and draws flat colours without it.
//!
//! The validator rejects entries without `license`, `source` or `author`,
//! licences outside [`crate::licence`] (share-alike, non-commercial,
//! no-derivatives, unknown) and files without a SHA-256.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::licence::{self, Obligation};

/// File name of the manifest in the repository (`assets/`).
pub const SOURCE_MANIFEST: &str = "materials.toml";
/// File name of the manifest copy inside a library folder.
pub const LIBRARY_MANIFEST: &str = "manifest.toml";
/// Credits written into a library folder.
pub const CREDITS_FILE: &str = "CREDITS.md";

/// Ground materials, in the order of the renderer's texture array layers
/// (spec §4.6: what the land classes are drawn with).
pub const GROUND_IDS: [&str; 9] = [
    "meadow",
    "farmland",
    "forest_floor",
    "rock",
    "scree",
    "gravel",
    "sand",
    "snow",
    "wetland",
];

/// Road materials, in layer order (`ocw_world::RoadSurface` codes).
pub const ROAD_IDS: [&str; 5] = ["asphalt", "asphalt_worn", "gravel_road", "paving", "dirt_road"];

/// Building materials (Phase 8 T4).
pub const BUILDING_IDS: [&str; 3] = ["roof_tiles", "plaster", "timber"];

/// What a material is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Ground,
    Road,
    Building,
}

/// One pinned source file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFile {
    /// A URL, or the member name inside the material's `archive`.
    pub file: String,
    /// SHA-256 of the file's bytes (lowercase hex).
    pub sha256: String,
}

/// One material.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub id: String,
    pub name: String,
    pub group: Group,
    /// Real-world size of one texture repeat (m).
    pub size_m: f32,
    #[serde(default)]
    pub license: String,
    /// The asset's page (where its licence was checked).
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub author: String,
    /// The source asset's own name (e.g. "Grass 004").
    #[serde(default)]
    pub title: String,
    /// A pinned archive (zip) whose members the files name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
    pub albedo: SourceFile,
    pub normal: SourceFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<SourceFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ao: Option<SourceFile>,
    /// Packed ambient occlusion (R), roughness (G), metalness (B).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arm: Option<SourceFile>,
}

impl Material {
    /// The source files with their role.
    pub fn files(&self) -> Vec<(&'static str, &SourceFile)> {
        let mut v = vec![("albedo", &self.albedo), ("normal", &self.normal)];
        for (role, f) in [
            ("roughness", &self.roughness),
            ("ao", &self.ao),
            ("arm", &self.arm),
        ] {
            if let Some(f) = f {
                v.push((role, f));
            }
        }
        v
    }

    /// File names of the converted textures.
    pub fn albedo_file(&self) -> String {
        format!("{}.albedo.ktx2", self.id)
    }

    pub fn normal_file(&self) -> String {
        format!("{}.normal.ktx2", self.id)
    }

    /// Where `f` is downloaded from.
    pub fn url_of<'a>(&'a self, f: &'a SourceFile) -> &'a str {
        self.archive.as_deref().unwrap_or(&f.file)
    }

    /// What the conversion depends on: a change re-converts the material.
    pub fn stamp(&self) -> String {
        let mut s = format!(
            "converter {} size {}",
            CONVERTER_VERSION, TEXTURE_SIZE
        );
        for (role, f) in self.files() {
            s.push_str(&format!("\n{role} {}", f.sha256));
        }
        s.push('\n');
        s
    }
}

/// Texture size the library is converted to (1K: about 3 mm per texel on
/// a 3 m tile).
pub const TEXTURE_SIZE: u32 = 1024;
/// Version of the conversion ([`crate::convert`] when built with the
/// `tools` feature); a change re-converts every material.
pub const CONVERTER_VERSION: u32 = 1;
/// KTX2 metadata key of the mean albedo ("r g b", linear).
pub const META_MEAN: &str = "OCWmeanLinearRGB";
/// KTX2 metadata key of the material id.
pub const META_ID: &str = "OCWmaterial";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    material: Vec<Material>,
}

/// The parsed manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialManifest {
    pub materials: Vec<Material>,
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl MaterialManifest {
    /// Parses and validates.
    pub fn parse(text: &str) -> Result<Self, Vec<String>> {
        let raw: Raw = toml::from_str(text).map_err(|e| vec![format!("materials: {e}")])?;
        let m = Self {
            materials: raw.material,
        };
        let errors = m.check();
        if errors.is_empty() {
            Ok(m)
        } else {
            Err(errors)
        }
    }

    /// Back to TOML (the library folder's copy).
    pub fn to_toml(&self) -> String {
        toml::to_string(&Raw {
            material: self.materials.clone(),
        })
        .expect("serialisable")
    }

    pub fn get(&self, id: &str) -> Option<&Material> {
        self.materials.iter().find(|m| m.id == id)
    }

    /// Every problem of the manifest (empty = valid).
    pub fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut ids = BTreeSet::new();
        for m in &self.materials {
            let id = &m.id;
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                errors.push(format!("material `{id}`: ids are lowercase a-z, 0-9, _"));
            }
            if !ids.insert(id.as_str()) {
                errors.push(format!("{id}: listed twice"));
            }
            for (key, value) in [
                ("license", &m.license),
                ("source", &m.source),
                ("author", &m.author),
            ] {
                if value.trim().is_empty() {
                    errors.push(format!(
                        "{id}: no `{key}` – every material needs a licence entry (spec §13)"
                    ));
                }
            }
            if !m.license.trim().is_empty()
                && let Err(why) = licence::check(&m.license)
            {
                errors.push(format!("{id}: licence `{}`: {why}", m.license));
            }
            if !m.source.trim().is_empty() && !m.source.starts_with("https://") {
                errors.push(format!("{id}: `source` must be the asset's https:// page"));
            }
            if !(m.size_m > 0.0 && m.size_m < 100.0) {
                errors.push(format!("{id}: size_m {} out of range", m.size_m));
            }
            if let Some(a) = &m.archive
                && !a.starts_with("https://")
            {
                errors.push(format!("{id}: archive must be an https:// URL"));
            }
            match (&m.roughness, &m.arm) {
                (None, None) => errors.push(format!("{id}: needs `roughness` or `arm`")),
                (Some(_), Some(_)) => errors.push(format!("{id}: `roughness` and `arm` both set")),
                _ => {}
            }
            if m.ao.is_some() && m.arm.is_some() {
                errors.push(format!("{id}: `ao` and `arm` both set"));
            }
            for (role, f) in m.files() {
                if !is_sha256(&f.sha256) {
                    errors.push(format!(
                        "{id}: {role} needs a sha256 (64 lowercase hex digits)"
                    ));
                }
                if m.archive.is_none() && !f.file.starts_with("https://") {
                    errors.push(format!(
                        "{id}: {role} `{}` is no https:// URL (and there is no archive)",
                        f.file
                    ));
                }
                if f.file.trim().is_empty() {
                    errors.push(format!("{id}: {role} has no file"));
                }
            }
        }
        errors
    }

    /// Ids the renderer needs that are missing (ground and road layers).
    pub fn missing_layers(&self) -> Vec<&'static str> {
        GROUND_IDS
            .iter()
            .chain(ROAD_IDS.iter())
            .filter(|id| self.get(id).is_none())
            .copied()
            .collect()
    }
}

/// SHA-256 of `bytes`, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Checks a downloaded file against its pin.
pub fn verify(what: &str, bytes: &[u8], expected: &str) -> Result<(), String> {
    let got = sha256_hex(bytes);
    if got == expected {
        Ok(())
    } else {
        Err(format!(
            "{what}: SHA-256 mismatch (expected {expected}, got {got}); the source changed – check its licence and page again, then update assets/materials.toml"
        ))
    }
}

/// One line of the credits screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    pub what: String,
    pub title: String,
    pub author: String,
    pub licence: String,
    pub source: String,
    pub attribution_required: bool,
}

impl Credit {
    pub fn line(&self) -> String {
        let title = if self.title.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.title)
        };
        format!(
            "{}{title}: {}, {}, {}",
            self.what, self.author, self.licence, self.source
        )
    }
}

/// Credits of the materials (we credit CC0 sources too, spec Phase 8A).
pub fn credits(m: &MaterialManifest) -> Vec<Credit> {
    m.materials
        .iter()
        .map(|x| Credit {
            what: x.name.clone(),
            title: x.title.clone(),
            author: x.author.clone(),
            licence: x.license.clone(),
            source: x.source.clone(),
            attribution_required: licence::check(&x.license) == Ok(Obligation::Attribution),
        })
        .collect()
}

/// `CREDITS.md` of a library folder.
pub fn credits_markdown(m: &MaterialManifest) -> String {
    let mut s = String::from(
        "# Material credits\n\nTiling PBR materials of Open Cycling World (spec §4.6, §13). \
         Converted to KTX2 (resized, compressed, ambient occlusion baked into the albedo) \
         by `ocw-assets materials fetch` from the pinned sources in `assets/materials.toml`.\n\n\
         | Material | Source asset | Author | Licence | Page |\n|---|---|---|---|---|\n",
    );
    for c in credits(m) {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            c.what, c.title, c.author, c.licence, c.source
        ));
    }
    s
}

/// Where the app looks for the library, in order: `$OCW_MATERIALS`,
/// beside the binary (`materials/`; Windows zip, test deployment), the
/// AppImage / Linux layout (`../share/ocw/materials`), the macOS bundle
/// (`../Resources/materials`), the checkout the binary was built from
/// (`assets/materials`, development builds), the working directory.
pub fn library_candidates(
    env: Option<&str>,
    exe: Option<&Path>,
    checkout: Option<&Path>,
) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(e) = env.filter(|e| !e.is_empty()) {
        v.push(PathBuf::from(e));
    }
    if let Some(dir) = exe.and_then(Path::parent) {
        v.push(dir.join("materials"));
        v.push(dir.join("../share/ocw/materials"));
        v.push(dir.join("../Resources/materials"));
    }
    if let Some(c) = checkout {
        v.push(c.join("assets/materials"));
    }
    v.push(PathBuf::from("assets/materials"));
    v
}

/// The first candidate with a library manifest.
pub fn find_library(candidates: &[PathBuf], exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|d| exists(&d.join(LIBRARY_MANIFEST)))
        .cloned()
}

/// A loaded material: its entry and the two textures.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub material: Material,
    pub albedo: crate::ktx2::Texture,
    pub normal: crate::ktx2::Texture,
    /// Mean albedo, linear RGB (far colour).
    pub mean: [f32; 3],
}

/// A loaded library folder.
#[derive(Debug, Clone)]
pub struct Library {
    pub dir: PathBuf,
    pub manifest: MaterialManifest,
    pub materials: Vec<Loaded>,
}

impl Library {
    pub fn get(&self, id: &str) -> Option<&Loaded> {
        self.materials.iter().find(|m| m.material.id == id)
    }
}

/// Loads (and checks) a library folder: the manifest copy and both
/// textures of every material, all the same size with a full mip chain.
pub fn load_library(dir: &Path) -> Result<Library, Vec<String>> {
    let text = std::fs::read_to_string(dir.join(LIBRARY_MANIFEST))
        .map_err(|e| vec![format!("{}: {e}", dir.join(LIBRARY_MANIFEST).display())])?;
    let manifest = MaterialManifest::parse(&text)?;
    let mut errors = Vec::new();
    let mut materials = Vec::new();
    for m in &manifest.materials {
        let load = |name: String, want: crate::ktx2::TexFormat| -> Result<crate::ktx2::Texture, String> {
            let bytes = std::fs::read(dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
            let t = crate::ktx2::read(&bytes).map_err(|e| format!("{name}: {e}"))?;
            if t.format != want {
                return Err(format!("{name}: {:?}, expected {want:?}", t.format));
            }
            if (t.width, t.height) != (TEXTURE_SIZE, TEXTURE_SIZE)
                || t.levels.len() as u32 != crate::ktx2::full_chain(TEXTURE_SIZE, TEXTURE_SIZE)
            {
                return Err(format!(
                    "{name}: {} × {} with {} levels, expected {TEXTURE_SIZE}² with a full mip chain",
                    t.width,
                    t.height,
                    t.levels.len()
                ));
            }
            Ok(t)
        };
        let albedo = load(m.albedo_file(), crate::ktx2::TexFormat::Bc3Srgb);
        let normal = load(m.normal_file(), crate::ktx2::TexFormat::Bc5);
        match (albedo, normal) {
            (Ok(albedo), Ok(normal)) => {
                let mean = mean_of(&albedo).unwrap_or([0.2, 0.2, 0.2]);
                materials.push(Loaded {
                    material: m.clone(),
                    albedo,
                    normal,
                    mean,
                });
            }
            (a, n) => {
                errors.extend(a.err());
                errors.extend(n.err());
            }
        }
    }
    if errors.is_empty() {
        Ok(Library {
            dir: dir.to_path_buf(),
            manifest,
            materials,
        })
    } else {
        Err(errors)
    }
}

/// The mean albedo stored by the converter.
pub fn mean_of(t: &crate::ktx2::Texture) -> Option<[f32; 3]> {
    let v: Vec<f32> = t
        .meta(META_MEAN)?
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// The shipped source manifest (`assets/materials.toml`).
pub const SHIPPED: &str = include_str!("../../../assets/materials.toml");

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn entry(extra: &str) -> String {
        format!(
            "[[material]]\nid = \"meadow\"\nname = \"Meadow\"\ngroup = \"ground\"\nsize_m = 1.4\n\
             archive = \"https://example.org/a.zip\"\n\
             albedo = {{ file = \"c.jpg\", sha256 = \"{H}\" }}\n\
             normal = {{ file = \"n.jpg\", sha256 = \"{H}\" }}\n\
             roughness = {{ file = \"r.jpg\", sha256 = \"{H}\" }}\n{extra}\n"
        )
    }

    const OK: &str =
        "license = \"CC0-1.0\"\nsource = \"https://example.org/meadow\"\nauthor = \"Someone\"";

    #[test]
    fn a_complete_entry_passes_and_round_trips() {
        let m = MaterialManifest::parse(&entry(OK)).unwrap();
        assert_eq!(m.materials.len(), 1);
        assert_eq!(m.materials[0].albedo_file(), "meadow.albedo.ktx2");
        // Manifest round trip (the library folder's copy).
        let back = MaterialManifest::parse(&m.to_toml()).unwrap();
        assert_eq!(back, m);
        assert!(m.missing_layers().contains(&"asphalt"));
    }

    /// Licence validator (spec Phase 8E): entries without licence, source
    /// or author, and share-alike / non-commercial / no-derivatives /
    /// unknown licences are rejected.
    #[test]
    fn materials_without_an_acceptable_licence_entry_are_rejected() {
        let errs = MaterialManifest::parse(&entry("")).unwrap_err();
        assert_eq!(errs.len(), 3, "{errs:?}");
        assert!(errs.iter().all(|e| e.contains("licence entry")));
        for (bad, why) in [
            ("CC-BY-NC-4.0", "non-commercial"),
            ("CC-BY-SA-4.0", "share-alike"),
            ("CC-BY-ND-4.0", "no-derivatives"),
            ("Royalty free", "unknown"),
        ] {
            let errs = MaterialManifest::parse(&entry(&OK.replace("CC0-1.0", bad))).unwrap_err();
            assert_eq!(errs.len(), 1, "{errs:?}");
            assert!(errs[0].contains(why), "{bad}: {errs:?}");
        }
        // Attribution licences are fine, and marked for the credits.
        let by = MaterialManifest::parse(&entry(&OK.replace("CC0-1.0", "CC-BY-4.0"))).unwrap();
        assert!(credits(&by)[0].attribution_required);
        let errs =
            MaterialManifest::parse(&entry(&OK.replace("https://example.org/meadow", "ambientcg")))
                .unwrap_err();
        assert!(errs[0].contains("https://"), "{errs:?}");
    }

    #[test]
    fn files_need_pins_and_a_roughness_source() {
        let no_hash = entry(OK).replacen(H, "abc", 1);
        let errs = MaterialManifest::parse(&no_hash).unwrap_err();
        assert!(errs[0].contains("albedo needs a sha256"), "{errs:?}");
        // Without an archive every file is a URL.
        let no_archive = entry(OK).replace("archive = \"https://example.org/a.zip\"\n", "");
        let errs = MaterialManifest::parse(&no_archive).unwrap_err();
        assert_eq!(errs.len(), 3, "{errs:?}");
        let no_rough = entry(OK).replace(&format!("roughness = {{ file = \"r.jpg\", sha256 = \"{H}\" }}\n"), "");
        let errs = MaterialManifest::parse(&no_rough).unwrap_err();
        assert!(errs[0].contains("roughness"), "{errs:?}");
        let twice = format!("{}{}", entry(OK), entry(OK));
        assert!(
            MaterialManifest::parse(&twice)
                .unwrap_err()
                .iter()
                .any(|e| e.contains("twice"))
        );
        assert!(MaterialManifest::parse(&format!("{}\ncolour = 1", entry(OK))).is_err());
    }

    /// The pure part of the fetch: the hash check.
    #[test]
    fn downloads_are_checked_against_their_pin() {
        let bytes = b"abc";
        let h = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(bytes), h);
        assert!(verify("x", bytes, h).is_ok());
        let e = verify("meadow albedo", b"abd", h).unwrap_err();
        assert!(e.contains("meadow albedo") && e.contains("mismatch"), "{e}");
        // The stamp changes with any pinned hash.
        let m = MaterialManifest::parse(&entry(OK)).unwrap();
        let mut other = m.materials[0].clone();
        other.normal.sha256 = H.replace('0', "f");
        assert_ne!(m.materials[0].stamp(), other.stamp());
    }

    #[test]
    fn the_library_is_found_beside_the_binary_first() {
        let exe = Path::new("/opt/ocw/usr/bin/ocw-desktop");
        let c = library_candidates(None, Some(exe), Some(Path::new("/src/ocw")));
        assert_eq!(c[0], PathBuf::from("/opt/ocw/usr/bin/materials"));
        assert!(c.contains(&PathBuf::from("/opt/ocw/usr/bin/../share/ocw/materials")));
        assert!(c.contains(&PathBuf::from("/src/ocw/assets/materials")));
        let c2 = library_candidates(Some("/x"), Some(exe), None);
        assert_eq!(c2[0], PathBuf::from("/x"));
        let found = find_library(&c, |p| p.starts_with("/src"));
        assert_eq!(found, Some(PathBuf::from("/src/ocw/assets/materials")));
        assert_eq!(find_library(&c, |_| false), None);
    }

    /// The shipped manifest is valid, CC0 throughout (as chosen in T1), and
    /// has every layer the renderer needs.
    #[test]
    fn the_shipped_manifest_is_complete() {
        let m = MaterialManifest::parse(SHIPPED).unwrap_or_else(|e| panic!("{e:#?}"));
        assert!(m.missing_layers().is_empty(), "{:?}", m.missing_layers());
        for id in BUILDING_IDS {
            assert!(m.get(id).is_some(), "{id}");
        }
        for x in &m.materials {
            let want = if GROUND_IDS.contains(&x.id.as_str()) {
                Group::Ground
            } else if ROAD_IDS.contains(&x.id.as_str()) {
                Group::Road
            } else {
                Group::Building
            };
            assert_eq!(x.group, want, "{}", x.id);
            assert_eq!(x.license, "CC0-1.0", "{}", x.id);
        }
        let md = credits_markdown(&m);
        assert_eq!(md.matches("| CC0-1.0 |").count(), m.materials.len());
        assert!(md.contains("ambientCG") && md.contains("Poly Haven"));
    }
}
