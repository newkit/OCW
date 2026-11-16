//! The run-time model loader (spec §14 Phase 15, 15D): an index of
//! `artwork/3dmodels/manifest.toml`, read once, and `get(id)`, parsed
//! lazily and cached.
//!
//! Sources:
//! - **embedded** (the default): `build.rs` generates the `include_bytes!`
//!   table from the manifest, so release builds, the AppImage and the
//!   browser need no asset paths (§2.5);
//! - **a folder** (desktop dev builds): `OCW_MODELS=artwork/3dmodels` or
//!   `ocw-desktop --models DIR` ([`use_folder`] before the first model is
//!   read); [`ModelLibrary::reload_changed`] re-reads changed files (the
//!   hot reload of 15D – the renderer's rebuild of the objects using them
//!   is a later step).
//!
//! The server and the headless tools use the same library (anchors,
//! bounds, rigs) without Bevy.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::glb;
use crate::kinds::Kind;
use crate::manifest::{Entry, Manifest};
use crate::model::Model;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/models.rs"));
}

/// The embedded manifest.
pub const MANIFEST: &str = include_str!("../../../artwork/3dmodels/manifest.toml");

/// The embedded model files: (path in `artwork/3dmodels/`, bytes).
pub fn embedded_files() -> &'static [(&'static str, &'static [u8])] {
    embedded::FILES
}

/// Where the files come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Embedded,
    /// A model folder (dev builds; native only).
    Folder(std::path::PathBuf),
}

/// A loaded model and the file it was read from.
#[derive(Debug, Clone)]
struct Cached {
    model: Arc<Model>,
    #[allow(dead_code)]
    stamp: Option<std::time::SystemTime>,
}

/// The model library.
#[derive(Debug)]
pub struct ModelLibrary {
    manifest: Manifest,
    source: Source,
    cache: Mutex<HashMap<String, Cached>>,
}

/// Why a model could not be had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryError(pub String);

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LibraryError {}

fn lfs_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(b"version https://git-lfs")
}

impl ModelLibrary {
    /// The library of the embedded files.
    pub fn embedded() -> Self {
        let manifest = Manifest::parse(MANIFEST)
            .unwrap_or_else(|e| panic!("artwork/3dmodels/manifest.toml: {e:?}"));
        Self {
            manifest,
            source: Source::Embedded,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The library of a model folder (its own `manifest.toml`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn folder(dir: &std::path::Path) -> Result<Self, LibraryError> {
        let path = dir.join(crate::manifest::MANIFEST);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| LibraryError(format!("{}: {e}", path.display())))?;
        let manifest = Manifest::parse(&text)
            .map_err(|e| LibraryError(format!("{}: {}", path.display(), e.join("; "))))?;
        Ok(Self {
            manifest,
            source: Source::Folder(dir.to_path_buf()),
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The entry of `id` (the file's path without `.glb`).
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.manifest.assets.iter().find(|e| e.id() == id)
    }

    /// Every entry of `kind`.
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Entry> {
        self.manifest.assets.iter().filter(move |e| e.kind == kind)
    }

    /// The file's bytes.
    pub fn bytes(&self, id: &str) -> Result<std::borrow::Cow<'static, [u8]>, LibraryError> {
        let e = self
            .entry(id)
            .ok_or_else(|| LibraryError(format!("no model `{id}` in the manifest")))?;
        let bytes: std::borrow::Cow<'static, [u8]> = match &self.source {
            Source::Embedded => embedded::FILES
                .iter()
                .find(|(f, _)| *f == e.file)
                .map(|(_, b)| std::borrow::Cow::Borrowed(*b))
                .ok_or_else(|| LibraryError(format!("{}: not embedded (missing file)", e.file)))?,
            #[cfg(not(target_arch = "wasm32"))]
            Source::Folder(dir) => std::fs::read(dir.join(&e.file))
                .map(std::borrow::Cow::Owned)
                .map_err(|err| LibraryError(format!("{}: {err}", e.file)))?,
            #[cfg(target_arch = "wasm32")]
            Source::Folder(_) => {
                return Err(LibraryError("no model folders in the browser".into()));
            }
        };
        if lfs_pointer(&bytes) {
            return Err(LibraryError(format!(
                "{}: a Git LFS pointer, not the model – run `git lfs pull` (spec §14 Phase 15, Q2)",
                e.file
            )));
        }
        Ok(bytes)
    }

    fn stamp(&self, id: &str) -> Option<std::time::SystemTime> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Source::Folder(dir) = &self.source {
            let e = self.entry(id)?;
            return std::fs::metadata(dir.join(&e.file))
                .and_then(|m| m.modified())
                .ok();
        }
        let _ = id;
        None
    }

    /// The model `id`, parsed once.
    pub fn get(&self, id: &str) -> Result<Arc<Model>, LibraryError> {
        if let Some(c) = self.cache.lock().expect("library lock").get(id) {
            return Ok(c.model.clone());
        }
        let stamp = self.stamp(id);
        let bytes = self.bytes(id)?;
        let model = Arc::new(glb::read(&bytes).map_err(|e| LibraryError(format!("{id}: {e}")))?);
        self.cache.lock().expect("library lock").insert(
            id.to_owned(),
            Cached {
                model: model.clone(),
                stamp,
            },
        );
        Ok(model)
    }

    /// Re-reads the cached models whose files changed (a model folder's
    /// hot reload); returns their ids.
    pub fn reload_changed(&self) -> Vec<String> {
        let ids: Vec<(String, Option<std::time::SystemTime>)> = self
            .cache
            .lock()
            .expect("library lock")
            .iter()
            .map(|(k, c)| (k.clone(), c.stamp))
            .collect();
        let mut changed = Vec::new();
        for (id, old) in ids {
            let now = self.stamp(&id);
            if now.is_some() && now != old {
                self.cache.lock().expect("library lock").remove(&id);
                if self.get(&id).is_ok() {
                    changed.push(id);
                }
            }
        }
        changed.sort();
        changed
    }
}

static FOLDER: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Uses a model folder instead of the embedded files (dev builds; call it
/// before the first model is read). Returns false when the library was
/// already in use.
pub fn use_folder(dir: impl Into<std::path::PathBuf>) -> bool {
    FOLDER.set(dir.into()).is_ok() && LIBRARY.get().is_none()
}

static LIBRARY: OnceLock<ModelLibrary> = OnceLock::new();

/// The app's library: the folder of [`use_folder`] or `OCW_MODELS` (if it
/// reads), else the embedded files.
pub fn library() -> &'static ModelLibrary {
    LIBRARY.get_or_init(|| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = FOLDER
                .get()
                .cloned()
                .or_else(|| std::env::var_os("OCW_MODELS").map(Into::into));
            if let Some(dir) = dir {
                match ModelLibrary::folder(&dir) {
                    Ok(l) => return l,
                    Err(e) => eprintln!("models: {e}; using the embedded models"),
                }
            }
        }
        ModelLibrary::embedded()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every manifest entry is embedded, loads and passes its kind's
    /// contract; the embedded table is the folder (15F).
    #[test]
    fn every_entry_loads_and_the_table_matches_the_folder() {
        let lib = ModelLibrary::embedded();
        assert!(!lib.manifest().assets.is_empty());
        for e in &lib.manifest().assets {
            let m = lib.get(e.id()).unwrap_or_else(|err| panic!("{err}"));
            assert!(m.triangles() > 0, "{}", e.file);
            // Cached: the same model.
            assert!(Arc::ptr_eq(&m, &lib.get(e.id()).unwrap()));
        }
        assert_eq!(embedded_files().len(), lib.manifest().assets.len());
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artwork/3dmodels");
        let disk = crate::manifest::read_dir_models(&dir).unwrap();
        assert_eq!(disk.len(), embedded_files().len());
        for (name, bytes) in &disk {
            let (_, b) = embedded_files()
                .iter()
                .find(|(f, _)| f == name)
                .unwrap_or_else(|| panic!("{name} not embedded"));
            assert_eq!(b, &bytes.as_slice(), "{name}");
        }
        let checked = crate::manifest::validate(
            MANIFEST,
            &embedded_files()
                .iter()
                .map(|(n, b)| (n.to_string(), b.to_vec()))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(checked.len(), lib.manifest().assets.len());
    }

    #[test]
    fn unknown_ids_and_lfs_pointers_are_clear_errors() {
        let lib = ModelLibrary::embedded();
        let e = lib.get("props/bollard-nowhere").unwrap_err();
        assert!(e.0.contains("no model"), "{e}");
        assert!(lfs_pointer(
            b"version https://git-lfs.github.com/spec/v1\noid sha256:00\n"
        ));
        assert!(!lfs_pointer(b"glTF"));
    }

    /// A model folder is read from disk and a changed file is re-read
    /// (the hot reload, headless).
    #[test]
    fn a_folder_library_reloads_changed_files() {
        let dir = std::env::temp_dir().join(format!("ocw-models-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("riders")).unwrap();
        let lib = ModelLibrary::embedded();
        let e = lib
            .entry(crate::export::RIDER.trim_end_matches(".glb"))
            .unwrap()
            .clone();
        let text = crate::export::manifest_text(std::slice::from_ref(&e));
        std::fs::write(dir.join("manifest.toml"), text).unwrap();
        let bytes = lib.bytes(e.id()).unwrap();
        std::fs::write(dir.join(&e.file), &*bytes).unwrap();
        let folder = ModelLibrary::folder(&dir).unwrap();
        let a = folder.get(e.id()).unwrap();
        assert_eq!(folder.reload_changed(), Vec::<String>::new());
        // An artist's export: the file changes.
        let mut m = (*a).clone();
        m.nodes[0].name = "rider".into();
        m.copyright = "changed".into();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join(&e.file), glb::write(&m)).unwrap();
        let f = std::fs::File::options()
            .append(true)
            .open(dir.join(&e.file))
            .unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(folder.reload_changed(), vec![e.id().to_string()]);
        assert_eq!(folder.get(e.id()).unwrap().copyright, "changed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
