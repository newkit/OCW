//! Loading a world package as the client and the server do (spec §4.7):
//! from a path or from the cache by id, verified on every load.
//!
//! **Policy for packages that are not signed by a trusted root** (spec
//! §12), decided per source:
//!
//! | Source | Default | Override |
//! |---|---|---|
//! | Cache id (`--world europe/liechtenstein`) | refuse | `--allow-unsigned` |
//! | Path (`--world my.ocwpkg`, a local build) | load with a warning | `--require-signed` refuses |
//!
//! A manifest whose hashes or signatures do not match, or a revoked key, is
//! always refused (that is damage or tampering, not missing trust). A
//! path without a manifest next to it (`<file>.manifest.json`) counts as
//! unsigned.
//!
//! A package signed by a **user key** nobody here trusts (spec §4.7 *User
//! keys*) is refused by both policies ([`VerifyError::UntrustedKey`]):
//! the apps first ask with the trust dialog ([`consent_needed`] gives what
//! it shows), and load it once the user trusted the key (or chose *Load
//! once*).

use std::path::{Path, PathBuf};

use ocw_world::WorldPackage;
use ocw_world::manifest::{
    Manifest, PackageIdentity, SignedManifest, TrustStatus, TrustStore, UnsignedPolicy,
    VerifyError, apply_policy,
};

use crate::cache::{MANIFEST_FILE, WorldCache, parse_spec};

/// How to load.
#[derive(Debug, Clone)]
pub struct LoadOptions {
    pub trust: TrustStore,
    pub cache: WorldCache,
    /// Accept unsigned / untrusted packages from the cache too (warning).
    pub allow_unsigned: bool,
    /// Refuse unsigned / untrusted packages from paths too.
    pub require_signed: bool,
    /// Current time (for the signature check).
    pub now_unix_s: i64,
}

impl LoadOptions {
    pub fn new(trust: TrustStore, cache: WorldCache) -> Self {
        Self {
            trust,
            cache,
            allow_unsigned: false,
            require_signed: false,
            now_unix_s: crate::now_unix_s(),
        }
    }

    fn policy(&self, from_cache: bool) -> UnsignedPolicy {
        let warn = if from_cache {
            self.allow_unsigned
        } else {
            !self.require_signed
        };
        if warn {
            UnsignedPolicy::Warn
        } else {
            UnsignedPolicy::Refuse
        }
    }
}

/// A loaded, verified package.
#[derive(Debug, Clone)]
pub struct LoadedWorld {
    pub package: WorldPackage,
    /// Id, version and content hash (local packages: `local/<name>@0`).
    pub identity: PackageIdentity,
    pub manifest: Option<Manifest>,
    pub trust: TrustStatus,
    /// The package file.
    pub path: PathBuf,
    /// The manifest file, if any.
    pub manifest_path: Option<PathBuf>,
}

impl LoadedWorld {
    /// One line for logs and the console.
    pub fn describe(&self) -> String {
        format!(
            "{} [{}] from {}",
            self.identity,
            self.trust,
            self.path.display()
        )
    }

    /// Credits to show (spec §13): the manifest's notices, or the OSM
    /// notice for packages without a manifest (they are built from OSM).
    pub fn attributions(&self) -> Vec<String> {
        match &self.manifest {
            Some(m) if !m.attribution.is_empty() => {
                m.attribution.iter().map(|a| a.notice.clone()).collect()
            }
            _ => vec!["© OpenStreetMap contributors".into()],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{0}: {1}")]
    Io(PathBuf, String),
    #[error("{0}: no such file, and no package with this id in the cache {1} (installed: {2})")]
    NotFound(String, PathBuf, String),
    #[error("{0}: {1}")]
    Verify(PathBuf, VerifyError),
    #[error("{0}: {1}")]
    Package(PathBuf, String),
}

/// The manifest that belongs to a package file: `<file>.manifest.json`, or
/// `manifest.json` in the same directory if it lists the file (a cache
/// entry or a published directory).
pub fn manifest_path_for(package: &Path) -> Option<PathBuf> {
    let mut side = package.as_os_str().to_owned();
    side.push(".manifest.json");
    let side = PathBuf::from(side);
    if side.is_file() {
        return Some(side);
    }
    let dir_manifest = package.parent()?.join(MANIFEST_FILE);
    let name = package.file_name()?.to_str()?;
    let text = std::fs::read_to_string(&dir_manifest).ok()?;
    let m = SignedManifest::parse(&text).ok()?;
    m.manifest.file(name).is_some().then_some(dir_manifest)
}

/// Whether loading `spec` needs the user's consent first (the trust
/// dialog, spec §4.7 *User keys*): `Ok(Some(request))` for an intact
/// package – its file matches the manifest – signed by a user key that
/// `opts.trust` does not hold; `Ok(None)` when it can be loaded (or is
/// left to the unsigned-package policy). A damaged package, a bad
/// signature or a revoked key is an error: refused, never asked about.
pub fn consent_needed(
    spec: &str,
    opts: &LoadOptions,
) -> Result<Option<crate::userkeys::TrustRequest>, LoadError> {
    let path = resolve(spec, opts)?;
    let Some(mp) = manifest_path_for(&path) else {
        return Ok(None);
    };
    let text =
        std::fs::read_to_string(&mp).map_err(|e| LoadError::Io(mp.clone(), e.to_string()))?;
    let signed = SignedManifest::parse(&text).map_err(|e| LoadError::Verify(mp.clone(), e))?;
    match crate::userkeys::decide(&signed, &opts.trust, opts.now_unix_s) {
        crate::userkeys::Decision::Ask(request) => {
            // Only an intact package is offered: its file must match.
            let bytes =
                std::fs::read(&path).map_err(|e| LoadError::Io(path.clone(), e.to_string()))?;
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            signed
                .manifest
                .verify_file(name, &bytes)
                .map_err(|e| LoadError::Verify(path.clone(), e))?;
            Ok(Some(*request))
        }
        crate::userkeys::Decision::Refuse(e) => Err(LoadError::Verify(mp, e)),
        _ => Ok(None),
    }
}

/// The package file `spec` names: an existing path, else the newest (or
/// the given) version of a cache id.
fn resolve(spec: &str, opts: &LoadOptions) -> Result<PathBuf, LoadError> {
    let path = Path::new(spec);
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if parse_spec(spec).is_ok()
        && let Some(entry) = opts.cache.resolve(spec)
    {
        return entry
            .package_path()
            .map_err(|e| LoadError::Io(entry.dir.clone(), e));
    }
    Err(LoadError::NotFound(
        spec.into(),
        opts.cache.root().to_path_buf(),
        "see world list".into(),
    ))
}

/// Loads `spec`: an existing path, else a cache id (`id` or `id@version`).
pub fn load_world(spec: &str, opts: &LoadOptions) -> Result<LoadedWorld, LoadError> {
    load_world_as(spec, opts, true)
}

/// Like [`load_world`] (verified the same way), but only the package's
/// skeleton is decoded ([`WorldPackage::skeleton_from_bytes`]): for
/// neighbours, whose tiles are read again when the rider comes near.
pub fn load_world_skeleton(spec: &str, opts: &LoadOptions) -> Result<LoadedWorld, LoadError> {
    load_world_as(spec, opts, false)
}

fn load_world_as(spec: &str, opts: &LoadOptions, full: bool) -> Result<LoadedWorld, LoadError> {
    let path = Path::new(spec);
    if path.is_file() {
        return load_path(path, opts.policy(false), opts, full);
    }
    if parse_spec(spec).is_ok()
        && let Some(entry) = opts.cache.resolve(spec)
    {
        let pkg = entry
            .package_path()
            .map_err(|e| LoadError::Io(entry.dir.clone(), e))?;
        let world = load_path(&pkg, opts.policy(true), opts, full)?;
        if (world.identity.id.as_str(), world.identity.version)
            != (entry.id.as_str(), entry.version)
        {
            return Err(LoadError::Verify(
                entry.manifest_path(),
                VerifyError::Malformed(format!(
                    "the manifest is for {}@{}, not {}@{}",
                    world.identity.id, world.identity.version, entry.id, entry.version
                )),
            ));
        }
        return Ok(world);
    }
    let installed: Vec<String> = opts
        .cache
        .list()
        .iter()
        .map(|e| format!("{}@{}", e.id, e.version))
        .collect();
    Err(LoadError::NotFound(
        spec.into(),
        opts.cache.root().to_path_buf(),
        if installed.is_empty() {
            "none".into()
        } else {
            installed.join(", ")
        },
    ))
}

fn load_path(
    path: &Path,
    policy: UnsignedPolicy,
    opts: &LoadOptions,
    full: bool,
) -> Result<LoadedWorld, LoadError> {
    let bytes = std::fs::read(path).map_err(|e| LoadError::Io(path.into(), e.to_string()))?;
    let manifest_path = manifest_path_for(path);
    let (manifest, trust, identity) = match &manifest_path {
        Some(mp) => {
            let text = std::fs::read_to_string(mp)
                .map_err(|e| LoadError::Io(mp.clone(), e.to_string()))?;
            let verify = |e| LoadError::Verify(mp.clone(), e);
            let signed = SignedManifest::parse(&text).map_err(verify)?;
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            signed
                .manifest
                .verify_file(name, &bytes)
                .map_err(|e| LoadError::Verify(path.into(), e))?;
            let trust = apply_policy(
                signed.verify_signature(&opts.trust, opts.now_unix_s),
                policy,
            )
            .map_err(verify)?;
            let identity = PackageIdentity {
                id: signed.manifest.id.clone(),
                version: signed.manifest.version,
                content_hash: ocw_world::manifest::sha256(&bytes),
            };
            (Some(signed.manifest), trust, identity)
        }
        None => {
            let trust = apply_policy(Err(VerifyError::Unsigned), policy)
                .map_err(|e| LoadError::Verify(path.into(), e))?;
            (None, trust, PackageIdentity::local("", &bytes))
        }
    };
    let package = if full {
        WorldPackage::from_bytes(&bytes)
    } else {
        WorldPackage::skeleton_from_bytes(&bytes)
    }
    .map_err(|e| LoadError::Package(path.into(), e))?;
    let identity = if manifest.is_none() {
        PackageIdentity::local(&package.name, &bytes)
    } else {
        identity
    };
    Ok(LoadedWorld {
        package,
        identity,
        manifest,
        trust,
        path: path.into(),
        manifest_path,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocw_world::manifest::{Attribution, RootKeyPair, SigningKeyPair};
    use ocw_world::package::{Tile, TileKey};

    pub(crate) const NOW: i64 = 1_800_000_000;

    pub(crate) fn package_bytes() -> Vec<u8> {
        let key = TileKey { x: 0, z: 0 };
        WorldPackage {
            name: "Mini".into(),
            origin: (47.0, 9.5),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: [(
                key,
                Tile {
                    key,
                    heights: vec![0.0; 4],
                    roads: Vec::new(),
                },
            )]
            .into(),
            graph: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            landuse: None,
            buildings: None,
            bridges: None,
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
            traffic: None,
            aero: None,
        }
        .to_bytes()
    }

    pub(crate) fn keys() -> (RootKeyPair, SigningKeyPair) {
        let root = RootKeyPair::from_seed("Test root", [5; 32]);
        let cert = root.certify(
            "s",
            SigningKeyPair::public_key_of([6; 32]),
            NOW - 10,
            NOW + 1000,
        );
        (root, SigningKeyPair::new([6; 32], cert).unwrap())
    }

    /// Writes `dir/mini.ocwpkg` and its manifest (signed or not).
    pub(crate) fn write_package(dir: &Path, signed: bool, sidecar: bool, version: u64) -> PathBuf {
        let bytes = package_bytes();
        let mut m = Manifest::new(
            "test/mini",
            "Mini",
            version,
            NOW,
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        m.add_file("mini.ocwpkg", &bytes).unwrap();
        let s = if signed {
            SignedManifest::signed(&m, &keys().1)
        } else {
            SignedManifest::unsigned(&m)
        };
        let p = dir.join("mini.ocwpkg");
        std::fs::write(&p, &bytes).unwrap();
        let mp = if sidecar {
            dir.join("mini.ocwpkg.manifest.json")
        } else {
            dir.join(MANIFEST_FILE)
        };
        std::fs::write(mp, s.to_json()).unwrap();
        p
    }

    fn opts(cache: &Path) -> LoadOptions {
        LoadOptions {
            now_unix_s: NOW,
            ..LoadOptions::new(TrustStore::new([keys().0.public()]), WorldCache::new(cache))
        }
    }

    #[test]
    fn paths_load_with_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_package(tmp.path(), true, true, 2);
        let o = opts(&tmp.path().join("cache"));
        let w = load_world(p.to_str().unwrap(), &o).unwrap();
        assert!(w.trust.is_trusted(), "{}", w.trust);
        assert_eq!(w.identity.id, "test/mini");
        assert_eq!(w.identity.version, 2);
        assert_eq!(
            w.identity.content_hash,
            ocw_world::manifest::sha256(&package_bytes())
        );
        assert!(w.attributions()[0].contains("OpenStreetMap"));
        assert!(w.describe().contains("test/mini@2"));
        // Unknown root: a warning by default, refused with --require-signed.
        let untrusting = LoadOptions {
            trust: TrustStore::default(),
            ..o.clone()
        };
        let w = load_world(p.to_str().unwrap(), &untrusting).unwrap();
        assert!(!w.trust.is_trusted());
        let strict = LoadOptions {
            require_signed: true,
            ..untrusting
        };
        let e = load_world(p.to_str().unwrap(), &strict).unwrap_err();
        assert!(matches!(
            e,
            LoadError::Verify(_, VerifyError::UnknownRoot(_))
        ));
        // A damaged package is refused whatever the policy.
        let mut bytes = std::fs::read(&p).unwrap();
        bytes[30] ^= 1;
        std::fs::write(&p, &bytes).unwrap();
        let e = load_world(p.to_str().unwrap(), &o).unwrap_err();
        assert!(
            matches!(e, LoadError::Verify(_, VerifyError::HashMismatch(_))),
            "{e}"
        );
        // No manifest at all: a local build, unsigned.
        let bare = tmp.path().join("bare.ocwpkg");
        std::fs::write(&bare, package_bytes()).unwrap();
        let w = load_world(bare.to_str().unwrap(), &o).unwrap();
        assert_eq!(w.identity.id, "local/mini");
        assert!(w.manifest_path.is_none() && !w.trust.is_trusted());
        assert!(
            load_world(
                bare.to_str().unwrap(),
                &LoadOptions {
                    require_signed: true,
                    ..o.clone()
                }
            )
            .is_err()
        );
        // manifest.json in the directory counts when it lists the file.
        let d2 = tmp.path().join("published");
        std::fs::create_dir(&d2).unwrap();
        let p2 = write_package(&d2, true, false, 2);
        assert_eq!(manifest_path_for(&p2), Some(d2.join(MANIFEST_FILE)));
        assert!(
            load_world(p2.to_str().unwrap(), &o)
                .unwrap()
                .trust
                .is_trusted()
        );
    }

    #[test]
    fn cache_ids_need_a_signature() {
        let tmp = tempfile::tempdir().unwrap();
        let o = opts(&tmp.path().join("cache"));
        for (version_dir, signed) in [(2, true), (3, false)] {
            let d = o.cache.dir("test/mini", version_dir);
            std::fs::create_dir_all(&d).unwrap();
            write_package(&d, signed, false, version_dir);
        }
        // A manifest in the wrong directory is refused.
        let d = o.cache.dir("test/mini", 9);
        std::fs::create_dir_all(&d).unwrap();
        write_package(&d, true, false, 4);
        let e = load_world("test/mini@9", &o).unwrap_err();
        assert!(e.to_string().contains("not test/mini@9"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
        // Highest version (3) is unsigned: refused from the cache ...
        let e = load_world("test/mini", &o).unwrap_err();
        assert!(
            matches!(e, LoadError::Verify(_, VerifyError::Unsigned)),
            "{e}"
        );
        // ... unless allowed; the signed one loads by explicit version.
        let allow = LoadOptions {
            allow_unsigned: true,
            ..o.clone()
        };
        assert!(!load_world("test/mini", &allow).unwrap().trust.is_trusted());
        assert!(load_world("test/mini@2", &o).unwrap().trust.is_trusted());
        let e = load_world("test/other", &o).unwrap_err();
        assert!(e.to_string().contains("test/mini@2, test/mini@3"), "{e}");
        assert!(matches!(
            load_world("no/such/file.ocwpkg", &o),
            Err(LoadError::NotFound(..))
        ));
    }

    /// Writes `dir/<name>.ocwpkg` signed by `key` (a user key).
    pub(crate) fn write_user_package(dir: &Path, name: &str, key: &SigningKeyPair) -> PathBuf {
        let bytes = package_bytes();
        let mut m = Manifest::new(
            &format!("user/{name}"),
            "Ada's Mini",
            1,
            NOW,
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        let file = format!("{name}.ocwpkg");
        m.add_file(&file, &bytes).unwrap();
        let p = dir.join(&file);
        std::fs::write(&p, &bytes).unwrap();
        std::fs::write(
            dir.join(format!("{file}.manifest.json")),
            SignedManifest::signed(&m, key).to_json(),
        )
        .unwrap();
        p
    }

    #[test]
    fn user_keys_need_consent_and_load_when_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        let (ada, key) = crate::userkeys::tests::user_key("Ada Rider", 61);
        let p = write_user_package(tmp.path(), "mini-ada", &key);
        let spec = p.to_str().unwrap();
        let o = opts(&tmp.path().join("cache"));
        // Unknown: refused by both policies, with a request for the dialog.
        let e = load_world(spec, &o).unwrap_err();
        assert!(
            matches!(e, LoadError::Verify(_, VerifyError::UntrustedKey(_))),
            "{e}"
        );
        let warn = LoadOptions {
            allow_unsigned: true,
            ..o.clone()
        };
        assert!(load_world(spec, &warn).is_err());
        let r = consent_needed(spec, &o).unwrap().expect("asks");
        assert_eq!(r.claimed_signer, "Ada Rider");
        assert_eq!(r.fingerprint(), ada.public().fingerprint());
        // Trusted: loads, unofficial; no question.
        let trusted = LoadOptions {
            trust: crate::userkeys::with_user_keys(o.trust.clone(), &[ada.public()]),
            ..o.clone()
        };
        assert_eq!(consent_needed(spec, &trusted).unwrap(), None);
        let w = load_world(spec, &trusted).unwrap();
        assert!(w.trust.is_trusted());
        assert!(w.trust.to_string().contains("UNOFFICIAL"), "{}", w.trust);
        // A file that does not match its manifest: refused, never asked.
        let mut bytes = std::fs::read(&p).unwrap();
        bytes[30] ^= 1;
        std::fs::write(&p, &bytes).unwrap();
        assert!(matches!(
            consent_needed(spec, &o),
            Err(LoadError::Verify(_, VerifyError::HashMismatch(_)))
        ));
        // Built-in roots need no consent.
        let official = write_package(tmp.path(), true, true, 2);
        assert_eq!(
            consent_needed(official.to_str().unwrap(), &o).unwrap(),
            None
        );
    }
}
