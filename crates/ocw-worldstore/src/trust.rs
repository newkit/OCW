//! Which root keys a build trusts (spec §4.7, §12).
//!
//! - **Production roots:** shipped with the client. None exist yet; they
//!   come with the distribution infrastructure (spec §4.2, Phase 7).
//! - **Development roots** (`keys/dev-roots.json`, public keys only, marked
//!   `dev_only`): trusted by debug builds, and by release builds only with
//!   `--trust-dev-root`. Their secret keys stay on the developers' machines.
//! - **The web preview root** (`keys/preview-roots.json`, spec §14 Phase 10
//!   B8a): the only root the browser app's preview build (GitHub Pages)
//!   trusts. Its secret keys stay on the build machine that signs the
//!   preview's worlds, never in the repository or CI.
//! - **Extra roots:** `--trust-root FILE` and every `*.pub.json` in
//!   `~/.config/ocw/trusted-roots/` (for private servers and own builds).
//! - **Revoked keys** ([`REVOKED_KEY_IDS`]) are refused even if a trusted
//!   root signed them.
//! - **User keys** (spec §4.7 *User keys*): the keys the user trusted in
//!   the trust dialog or with `world trust add` ([`crate::userkeys`]),
//!   appended after all of the above and marked as user keys (their
//!   packages are *unofficial*); a user key never replaces a built-in
//!   root. Plus the keys accepted with *Load once* in this run.

use std::path::{Path, PathBuf};

use ocw_world::manifest::{Hex, RootKey, TrustStore};

/// Development root public keys (a JSON array of root public key objects).
pub const DEV_ROOTS_JSON: &str = include_str!("../keys/dev-roots.json");

/// The web preview's root public keys (spec §14 Phase 10 B8a; a JSON array
/// like [`DEV_ROOTS_JSON`]).
pub const PREVIEW_ROOTS_JSON: &str = include_str!("../keys/preview-roots.json");

/// Production root public keys shipped with the client (none yet).
pub const PRODUCTION_ROOTS: &[&str] = &[];

/// Ids of revoked root or signing keys (hex).
pub const REVOKED_KEY_IDS: &[&str] = &[];

/// The development roots, marked `dev_only`.
pub fn dev_roots() -> Vec<RootKey> {
    parse_roots(DEV_ROOTS_JSON)
        .expect("keys/dev-roots.json is valid")
        .into_iter()
        .map(|mut k| {
            k.dev_only = true;
            k
        })
        .collect()
}

/// The web preview's roots (B8a): not development roots, not production
/// roots – trusted only by the preview build of the browser app.
pub fn preview_roots() -> Vec<RootKey> {
    parse_roots(PREVIEW_ROOTS_JSON).expect("keys/preview-roots.json is valid")
}

/// Parses a JSON array of root public keys.
pub fn parse_roots(json: &str) -> Result<Vec<RootKey>, String> {
    let values: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    values
        .iter()
        .map(|v| RootKey::parse(&v.to_string()))
        .collect()
}

/// `<config dir>/ocw/trusted-roots` (none in the browser).
#[cfg(not(target_arch = "wasm32"))]
pub fn user_roots_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ocw/trusted-roots"))
}

/// `<config dir>/ocw/trusted-roots` (none in the browser).
#[cfg(target_arch = "wasm32")]
pub fn user_roots_dir() -> Option<PathBuf> {
    None
}

/// What to trust.
#[derive(Debug, Clone, Default)]
pub struct TrustConfig {
    /// Trust the development roots (debug builds, `--trust-dev-root`).
    pub dev_roots: bool,
    /// Root public key files (`--trust-root`).
    pub root_files: Vec<PathBuf>,
    /// A directory of `*.pub.json` root files (default [`user_roots_dir`]).
    pub roots_dir: Option<PathBuf>,
    /// The user's trusted keys file (default
    /// [`crate::userkeys::default_path`]; a corrupt file trusts none).
    pub user_keys_file: Option<PathBuf>,
    /// More trusted user keys (the browser's `localStorage` list).
    pub user_keys: Vec<RootKey>,
    /// Also the keys accepted with *Load once* in this run.
    pub session_keys: bool,
}

impl TrustConfig {
    /// The default of this build: development roots in debug builds, the
    /// user's trusted-roots directory.
    pub fn for_build() -> Self {
        Self {
            dev_roots: cfg!(debug_assertions),
            root_files: Vec::new(),
            roots_dir: user_roots_dir(),
            user_keys_file: user_keys_path(),
            user_keys: Vec::new(),
            session_keys: true,
        }
    }

    /// The built-in roots and the user's keys (see the module docs).
    pub fn trust_store(&self) -> Result<TrustStore, String> {
        let builtin = self.builtin_store()?;
        let mut user = self.file_keys();
        user.extend(self.user_keys.iter().cloned());
        if self.session_keys {
            user.extend(crate::userkeys::once_roots());
        }
        Ok(crate::userkeys::with_user_keys(builtin, &user))
    }

    /// The trusted keys file's keys (none if it is missing or corrupt).
    fn file_keys(&self) -> Vec<RootKey> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = &self.user_keys_file {
            let (keys, _) = crate::userkeys::UserKeys::load(crate::userkeys::read_file(p));
            return keys.roots();
        }
        Vec::new()
    }

    /// The roots that are not user keys: production, development, preview
    /// and extra roots (the user key list can neither remove nor replace
    /// them).
    pub fn builtin_store(&self) -> Result<TrustStore, String> {
        let mut roots: Vec<RootKey> = PRODUCTION_ROOTS
            .iter()
            .map(|j| RootKey::parse(j))
            .collect::<Result<_, _>>()?;
        if self.dev_roots {
            roots.extend(dev_roots());
        }
        let mut files = self.root_files.clone();
        if let Some(dir) = &self.roots_dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            let mut found: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.to_string_lossy().ends_with(".pub.json"))
                .collect();
            found.sort();
            files.extend(found);
        }
        for f in files {
            roots.push(read_root(&f)?);
        }
        let mut seen = std::collections::BTreeSet::new();
        roots.retain(|r| seen.insert(r.key_id));
        let revoked = REVOKED_KEY_IDS
            .iter()
            .map(|h| Hex::parse(h))
            .collect::<Result<_, _>>()?;
        Ok(TrustStore { roots, revoked })
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn user_keys_path() -> Option<PathBuf> {
    crate::userkeys::default_path()
}

#[cfg(target_arch = "wasm32")]
fn user_keys_path() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn read_root(path: &Path) -> Result<RootKey, String> {
    crate::keys::read_root_public(path)
}

#[cfg(target_arch = "wasm32")]
fn read_root(path: &Path) -> Result<RootKey, String> {
    Err(format!("{}: no key files in the browser", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::manifest::RootKeyPair;

    #[test]
    fn built_in_lists_are_valid() {
        for k in dev_roots() {
            assert!(k.dev_only);
            assert!(k.name.to_lowercase().contains("development"), "{}", k.name);
        }
        for j in PRODUCTION_ROOTS {
            RootKey::parse(j).unwrap();
        }
        for h in REVOKED_KEY_IDS {
            Hex::<8>::parse(h).unwrap();
        }
        // The web preview's root (B8a): one key, not a development root,
        // none of the development or production keys.
        let preview = preview_roots();
        assert_eq!(preview.len(), 1);
        for k in &preview {
            assert!(!k.dev_only && k.name.contains("preview"), "{}", k.name);
            assert!(dev_roots().iter().all(|d| d.key_id != k.key_id));
            assert!(
                PRODUCTION_ROOTS
                    .iter()
                    .all(|j| RootKey::parse(j).unwrap().key_id != k.key_id)
            );
        }
    }

    #[test]
    fn trust_store_from_files_and_dir() {
        let dir = tempfile::tempdir().unwrap();
        let a = RootKeyPair::from_seed("a", [1; 32]).public();
        let b = RootKeyPair::from_seed("b", [2; 32]).public();
        let fa = dir.path().join("a.json");
        std::fs::write(&fa, a.to_json()).unwrap();
        let sub = dir.path().join("roots");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("b.pub.json"), b.to_json()).unwrap();
        std::fs::write(sub.join("ignored.txt"), "x").unwrap();
        let t = TrustConfig {
            dev_roots: false,
            root_files: vec![fa.clone()],
            roots_dir: Some(sub.clone()),
            ..TrustConfig::default()
        }
        .trust_store()
        .unwrap();
        assert_eq!(t.roots, [a.clone(), b]);
        // Development roots only when asked for.
        let dev = TrustConfig {
            dev_roots: true,
            ..TrustConfig::default()
        }
        .trust_store()
        .unwrap();
        assert_eq!(dev.roots.len(), dev_roots().len());
        // A broken root file is an error, not silently ignored.
        std::fs::write(sub.join("bad.pub.json"), "{}").unwrap();
        assert!(
            TrustConfig {
                roots_dir: Some(sub),
                ..TrustConfig::default()
            }
            .trust_store()
            .is_err()
        );
        assert!(parse_roots("[]").unwrap().is_empty());
    }

    #[test]
    fn user_keys_come_after_the_built_in_roots() {
        use crate::userkeys::UserKeys;
        let dir = tempfile::tempdir().unwrap();
        let official = RootKeyPair::from_seed("own root", [5; 32]).public();
        let ada = RootKeyPair::from_seed("Ada Rider", [6; 32]).public();
        let fo = dir.path().join("own.pub.json");
        std::fs::write(&fo, official.to_json()).unwrap();
        let mut keys = UserKeys::default();
        keys.add(&ada, None, None, 0, &TrustStore::default())
            .unwrap();
        // The file also names the built-in root (edited by hand): ignored.
        let mut both = keys.clone();
        both.add(&official, None, None, 0, &TrustStore::default())
            .unwrap();
        let file = dir.path().join(crate::userkeys::FILE_NAME);
        std::fs::write(&file, both.to_json()).unwrap();
        let config = TrustConfig {
            root_files: vec![fo],
            user_keys_file: Some(file.clone()),
            ..TrustConfig::default()
        };
        let t = config.trust_store().unwrap();
        assert_eq!(t.roots.len(), 2);
        assert!(!t.root(official.key_id).unwrap().user, "stays built in");
        assert!(t.root(ada.key_id).unwrap().user);
        assert_eq!(
            &config.builtin_store().unwrap().roots,
            std::slice::from_ref(&official)
        );
        // A corrupt file: no user keys, the built-in roots still work.
        std::fs::write(&file, "{ broken").unwrap();
        let t = config.trust_store().unwrap();
        assert_eq!(t.roots, [official]);
    }
}
