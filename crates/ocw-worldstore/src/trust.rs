//! Which root keys a build trusts (spec §4.7, §12).
//!
//! - **Production roots:** shipped with the client. None exist yet; they
//!   come with the distribution infrastructure (spec §4.2, Phase 7).
//! - **Development roots** (`keys/dev-roots.json`, public keys only, marked
//!   `dev_only`): trusted by debug builds, and by release builds only with
//!   `--trust-dev-root`. Their secret keys stay on the developers' machines.
//! - **Extra roots:** `--trust-root FILE` and every `*.pub.json` in
//!   `~/.config/ocw/trusted-roots/` (for private servers and own builds).
//! - **Revoked keys** ([`REVOKED_KEY_IDS`]) are refused even if a trusted
//!   root signed them.

use std::path::{Path, PathBuf};

use ocw_world::manifest::{Hex, RootKey, TrustStore};

/// Development root public keys (a JSON array of root public key objects).
pub const DEV_ROOTS_JSON: &str = include_str!("../keys/dev-roots.json");

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

/// Parses a JSON array of root public keys.
pub fn parse_roots(json: &str) -> Result<Vec<RootKey>, String> {
    let values: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    values
        .iter()
        .map(|v| RootKey::parse(&v.to_string()))
        .collect()
}

/// `<config dir>/ocw/trusted-roots`.
pub fn user_roots_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ocw/trusted-roots"))
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
}

impl TrustConfig {
    /// The default of this build: development roots in debug builds, the
    /// user's trusted-roots directory.
    pub fn for_build() -> Self {
        Self {
            dev_roots: cfg!(debug_assertions),
            root_files: Vec::new(),
            roots_dir: user_roots_dir(),
        }
    }

    pub fn trust_store(&self) -> Result<TrustStore, String> {
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

fn read_root(path: &Path) -> Result<RootKey, String> {
    crate::keys::read_root_public(path)
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
}
