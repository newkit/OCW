//! Local package cache (spec §4.7): packages are downloaded once and then
//! ridden offline.
//!
//! ```text
//! <cache>/                          $OCW_WORLD_CACHE or <data dir>/ocw/worlds
//!   europe/liechtenstein/           package id (path segments)
//!     3/                            version (build number)
//!       manifest.json               signed manifest
//!       liechtenstein.ocwpkg        the files it lists
//!   .partial/europe%liechtenstein@4/  a download in progress (resumable)
//! ```
//!
//! An entry is only moved into place after all its files were verified, so
//! a finished directory is always complete; it is verified again on load.

use std::path::{Path, PathBuf};

use ocw_world::manifest::{SignedManifest, validate_id};

/// File name of the manifest inside a cache entry.
pub const MANIFEST_FILE: &str = "manifest.json";
const PARTIAL_DIR: &str = ".partial";

#[derive(Debug, Clone)]
pub struct WorldCache {
    root: PathBuf,
}

/// One installed package version.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub id: String,
    pub version: u64,
    pub dir: PathBuf,
}

impl CacheEntry {
    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join(MANIFEST_FILE)
    }

    /// Reads and parses the manifest (no signature check).
    pub fn manifest(&self) -> Result<SignedManifest, String> {
        let p = self.manifest_path();
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        SignedManifest::parse(&text).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// The package file, as the manifest names it.
    pub fn package_path(&self) -> Result<PathBuf, String> {
        let m = self.manifest()?;
        let f = m
            .manifest
            .package_file()
            .ok_or("the manifest lists no package file")?;
        Ok(self.dir.join(&f.path))
    }
}

/// `id` or `id@version`.
pub fn parse_spec(spec: &str) -> Result<(&str, Option<u64>), String> {
    let (id, version) = match spec.split_once('@') {
        Some((id, v)) => (
            id,
            Some(
                v.parse::<u64>()
                    .map_err(|_| format!("bad version in {spec:?}"))?,
            ),
        ),
        None => (spec, None),
    };
    validate_id(id)?;
    Ok((id, version))
}

impl WorldCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$OCW_WORLD_CACHE`, else `<data dir>/ocw/worlds`
    /// (`~/.local/share/ocw/worlds` on Linux).
    pub fn default_root() -> PathBuf {
        if let Some(d) = std::env::var_os("OCW_WORLD_CACHE") {
            return PathBuf::from(d);
        }
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ocw/worlds")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory of `id` at `version` (`id` must be valid).
    pub fn dir(&self, id: &str, version: u64) -> PathBuf {
        self.root.join(id).join(version.to_string())
    }

    /// Where a download of `id` at `version` collects its files.
    pub fn partial_dir(&self, id: &str, version: u64) -> PathBuf {
        self.root
            .join(PARTIAL_DIR)
            .join(format!("{}@{version}", id.replace('/', "%")))
    }

    /// All installed entries, sorted by id and version.
    pub fn list(&self) -> Vec<CacheEntry> {
        let mut out = Vec::new();
        self.walk(&self.root, "", 0, &mut out);
        out.sort_by(|a, b| a.id.cmp(&b.id).then(a.version.cmp(&b.version)));
        out
    }

    fn walk(&self, dir: &Path, id: &str, depth: usize, out: &mut Vec<CacheEntry>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !e.path().is_dir() {
                continue;
            }
            if let Ok(version) = name.parse::<u64>()
                && !id.is_empty()
                && e.path().join(MANIFEST_FILE).is_file()
            {
                out.push(CacheEntry {
                    id: id.to_string(),
                    version,
                    dir: e.path(),
                });
                continue;
            }
            let sub = if id.is_empty() {
                name.clone()
            } else {
                format!("{id}/{name}")
            };
            if depth < 8 && validate_id(&sub).is_ok() {
                self.walk(&e.path(), &sub, depth + 1, out);
            }
        }
    }

    /// `id` (its highest version) or `id@version`.
    pub fn resolve(&self, spec: &str) -> Option<CacheEntry> {
        let (id, version) = parse_spec(spec).ok()?;
        self.list()
            .into_iter()
            .filter(|e| e.id == id && version.is_none_or(|v| v == e.version))
            .max_by_key(|e| e.version)
    }

    /// Moves a complete, verified download directory into place
    /// (replacing an older copy of the same version).
    pub fn commit(&self, staged: &Path, id: &str, version: u64) -> Result<CacheEntry, String> {
        validate_id(id)?;
        let dir = self.dir(id, version);
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::rename(staged, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(CacheEntry {
            id: id.into(),
            version,
            dir,
        })
    }

    /// Removes an installed version.
    pub fn remove(&self, id: &str, version: u64) -> Result<(), String> {
        validate_id(id)?;
        let dir = self.dir(id, version);
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(cache: &WorldCache, id: &str, version: u64) {
        let d = cache.dir(id, version);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(MANIFEST_FILE), "{}").unwrap();
    }

    #[test]
    fn layout_listing_and_resolving() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = WorldCache::new(tmp.path());
        entry(&cache, "europe/liechtenstein", 3);
        entry(&cache, "europe/liechtenstein", 12);
        entry(&cache, "test", 1);
        // Not entries: no manifest, hidden, bad names, partial downloads.
        std::fs::create_dir_all(cache.dir("europe/empty", 1)).unwrap();
        std::fs::create_dir_all(tmp.path().join("Bad Name/1")).unwrap();
        std::fs::write(tmp.path().join("Bad Name/1").join(MANIFEST_FILE), "").unwrap();
        let partial = cache.partial_dir("europe/liechtenstein", 13);
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join(MANIFEST_FILE), "").unwrap();
        assert!(partial.starts_with(tmp.path().join(".partial")));
        let list: Vec<(String, u64)> = cache
            .list()
            .into_iter()
            .map(|e| (e.id, e.version))
            .collect();
        assert_eq!(
            list,
            [
                ("europe/liechtenstein".to_string(), 3),
                ("europe/liechtenstein".to_string(), 12),
                ("test".to_string(), 1)
            ]
        );
        assert_eq!(cache.resolve("europe/liechtenstein").unwrap().version, 12);
        assert_eq!(cache.resolve("europe/liechtenstein@3").unwrap().version, 3);
        assert!(cache.resolve("europe/liechtenstein@4").is_none());
        assert!(cache.resolve("europe").is_none());
        assert!(cache.resolve("../x").is_none());
        assert!(parse_spec("a@b").is_err());
        assert_eq!(parse_spec("a/b@7"), Ok(("a/b", Some(7))));
        // Commit replaces a version atomically; remove deletes it.
        let staged = tmp.path().join("staged");
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(staged.join(MANIFEST_FILE), "new").unwrap();
        let e = cache.commit(&staged, "test", 1).unwrap();
        assert_eq!(std::fs::read_to_string(e.manifest_path()).unwrap(), "new");
        assert!(!staged.exists());
        cache.remove("test", 1).unwrap();
        assert!(cache.resolve("test").is_none());
    }
}
