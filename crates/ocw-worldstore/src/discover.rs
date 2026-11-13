//! Which worlds can be ridden here (spec §3.2.1 World → Change world,
//! §4.7): the installed packages of the cache, the package given with
//! `--world`, the packages next to it (the kiosk installs several in one
//! directory) and package files in further directories.
//!
//! Each package is described **without loading it**: the manifest (id,
//! name, version, date, credits, file size) and a summary of the package
//! file ([`ocw_world::summary`]: name, routes, region, time zone) that
//! skips the bulky sections. The manifest's signature is checked (cheap:
//! the manifest only); the package's hash is checked when it is ridden
//! ([`crate::load_world`]), so a damaged file is listed as such only if
//! its size or layout gives it away – the switch then refuses it and says
//! why.
//!
//! **Duplicates:** a file reached through two directories is listed once
//! (canonical path); packages with a manifest are one world per id, the
//! highest version, on a tie the one given with `--world`, then the cache,
//! then a file. Packages without a manifest are told apart by their file.
//!
//! **Parts:** packages whose regions meet at a border (boxes touching,
//! hardly any area shared, spec §2.2.2) are one world with parts: riding
//! one joins the installed neighbours anyway ([`crate::neighbourhood`]).
//! Two cuts of the same area (a test build next to the release) stay two
//! worlds.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ocw_world::manifest::{Attribution, SignedManifest, TrustStore, VerifyError};
use ocw_world::summary::{PackageSummary, summarize_file};

use crate::cache::WorldCache;
use crate::manifest_path_for;
use crate::neighbours::TOUCH_TOLERANCE_DEG;

/// Regions sharing less than this part of the smaller one are neighbours.
pub const PART_SHARED_MAX: f64 = 0.1;

/// Where an entry was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// The package given with `--world` (or ridden now).
    Given,
    /// The world cache (`<data dir>/ocw/worlds/<id>/<version>/`).
    Cache,
    /// Next to the given package.
    Sibling,
    /// In one of the searched directories.
    Folder,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Given => "given",
            Self::Cache => "installed",
            Self::Sibling => "next to it",
            Self::Folder => "file",
        }
    }
}

/// Whether a package can be trusted, as far as the list can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Signed by a trusted root (the signer's name).
    Verified(String),
    /// No manifest, or a manifest without a signature.
    Unsigned,
    /// Signed, but not by a trusted root (or the key is not valid).
    Untrusted(String),
    /// Cannot be ridden: not a package, damaged, a broken manifest.
    Broken(String),
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Verified(_) => "verified",
            Self::Unsigned => "unsigned",
            Self::Untrusted(_) => "untrusted",
            Self::Broken(_) => "broken",
        }
    }

    pub fn is_broken(&self) -> bool {
        matches!(self, Self::Broken(_))
    }
}

/// One world package found.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldEntry {
    /// What `--world` and `ride.switch --world` take: a path or `id@version`.
    pub spec: String,
    /// The manifest's id, or `local/<file name>` without one.
    pub id: String,
    pub version: u64,
    /// The region's name for people (manifest, else package, else file).
    pub name: String,
    pub path: Option<PathBuf>,
    pub source: Source,
    pub size_bytes: u64,
    /// When it was built (manifest).
    pub created_unix_s: Option<i64>,
    pub status: Status,
    /// Why it cannot be ridden with the current options (broken, or not
    /// signed where a signature is required); `None`: rideable.
    pub blocked: Option<String>,
    /// The package file's summary (`None` when unreadable).
    pub summary: Option<PackageSummary>,
    pub attributions: Vec<Attribution>,
    /// Neighbouring packages that continue this world (spec §2.2.2).
    pub parts: Vec<WorldEntry>,
    /// Other copies of the same id that are not listed (older versions,
    /// the same package elsewhere).
    pub also: Vec<String>,
}

impl WorldEntry {
    pub fn routes(&self) -> usize {
        self.summary.as_ref().map_or(0, |s| s.routes.len())
    }

    /// The first route (what `--world` alone rides, else a free ride).
    pub fn first_route(&self) -> Option<&str> {
        self.summary
            .as_ref()
            .and_then(|s| s.routes.first())
            .map(|r| r.name.as_str())
    }

    pub fn has_graph(&self) -> bool {
        self.summary.as_ref().is_some_and(|s| s.has_graph)
    }

    pub fn is_rideable(&self) -> bool {
        self.blocked.is_none()
    }

    /// The covered area (km²): the region, else the terrain tiles.
    pub fn area_km2(&self) -> Option<f64> {
        let s = self.summary.as_ref()?;
        Some(match &s.region {
            Some(r) => r.area_km2(),
            None => s.terrain_km2(),
        })
    }

    /// "v3, 30 MB, verified".
    pub fn detail(&self) -> String {
        let v = if self.id.starts_with("local/") {
            "local".to_string()
        } else {
            format!("v{}", self.version)
        };
        format!(
            "{v}, {}, {}",
            megabytes(self.size_bytes),
            self.status.label()
        )
    }

    /// The whole world: this package and its parts.
    pub fn members(&self) -> impl Iterator<Item = &WorldEntry> {
        std::iter::once(self).chain(self.parts.iter())
    }
}

/// "30 MB" (whole MB from 10 MB, else one decimal).
pub fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb >= 9.95 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

/// Where to look and how to judge what is found.
#[derive(Debug, Clone)]
pub struct Discovery {
    pub cache: WorldCache,
    /// `--world` (a path or a cache id), or the package ridden now.
    pub given: Option<String>,
    /// Further directories with package files (and one level below).
    pub dirs: Vec<PathBuf>,
    pub trust: TrustStore,
    pub now_unix_s: i64,
    /// As the load options: cached packages may be unsigned / paths must be signed.
    pub allow_unsigned: bool,
    pub require_signed: bool,
}

/// Package summaries by file, so opening the menu again reads only
/// changed files.
#[derive(Debug, Clone, Default)]
pub struct SummaryMemo(HashMap<PathBuf, Memo>);

/// A file's size and time, and its summary (or why it has none).
type Memo = (u64, Option<SystemTime>, Result<PackageSummary, String>);

impl SummaryMemo {
    fn get(&mut self, path: &Path) -> Result<PackageSummary, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let key = (meta.len(), meta.modified().ok());
        if let Some((len, t, r)) = self.0.get(path)
            && (*len, *t) == key
        {
            return r.clone();
        }
        let r = summarize_file(path);
        self.0.insert(path.to_path_buf(), (key.0, key.1, r.clone()));
        r
    }
}

/// A package file and how it was found, before judging it.
struct Found {
    path: PathBuf,
    spec: String,
    source: Source,
    manifest: Option<Result<SignedManifest, String>>,
}

/// Lists the worlds (see the module docs): the given one first, then by
/// name; broken packages at the end.
pub fn discover(d: &Discovery, memo: &mut SummaryMemo) -> Vec<WorldEntry> {
    let mut found: Vec<Found> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut add = |found: &mut Vec<Found>, path: PathBuf, spec: String, source, manifest| {
        let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.contains(&canon) {
            return;
        }
        seen.push(canon);
        found.push(Found {
            path,
            spec,
            source,
            manifest,
        });
    };
    let sidecar = |p: &Path| {
        manifest_path_for(p).map(|mp| {
            std::fs::read_to_string(&mp)
                .map_err(|e| format!("{}: {e}", mp.display()))
                .and_then(|t| SignedManifest::parse(&t).map_err(|e| format!("manifest: {e}")))
        })
    };
    // The given package: a file, or an id of the cache.
    let mut given_dir = None;
    if let Some(g) = &d.given {
        let p = Path::new(g);
        if p.is_file() {
            given_dir = p.parent().map(Path::to_path_buf);
            add(
                &mut found,
                p.to_path_buf(),
                g.clone(),
                Source::Given,
                sidecar(p),
            );
        } else if let Some(e) = d.cache.resolve(g) {
            let m = e.manifest();
            let path = e
                .package_path()
                .unwrap_or_else(|_| e.dir.join("package.ocwpkg"));
            add(
                &mut found,
                path,
                format!("{}@{}", e.id, e.version),
                Source::Given,
                Some(m),
            );
        }
    }
    // The cache: every installed version (duplicates are folded below).
    for e in d.cache.list() {
        let m = e.manifest();
        let path = match &m {
            Ok(sm) => sm
                .manifest
                .package_file()
                .map_or_else(|| e.dir.join("?"), |f| e.dir.join(&f.path)),
            Err(_) => e.dir.join("?"),
        };
        add(
            &mut found,
            path,
            format!("{}@{}", e.id, e.version),
            Source::Cache,
            Some(m),
        );
    }
    // Next to the given package, then the other directories.
    let mut dirs: Vec<(PathBuf, Source, usize)> = Vec::new();
    if let Some(dir) = given_dir {
        dirs.push((dir, Source::Sibling, 0));
    }
    dirs.extend(d.dirs.iter().map(|p| (p.clone(), Source::Folder, 1)));
    for (dir, source, depth) in dirs {
        let mut files = Vec::new();
        package_files(&dir, depth, &mut files);
        files.sort();
        for f in files {
            let spec = f.display().to_string();
            let m = sidecar(&f);
            add(&mut found, f, spec, source, m);
        }
    }
    let entries: Vec<WorldEntry> = found.into_iter().map(|f| judge(d, memo, f)).collect();
    group(fold_duplicates(entries))
}

fn package_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                package_files(&p, depth - 1, out);
            }
        } else if p.extension().is_some_and(|x| x == "ocwpkg") {
            out.push(p);
        }
    }
}

/// Describes one package: summary, manifest, status.
fn judge(d: &Discovery, memo: &mut SummaryMemo, f: Found) -> WorldEntry {
    let stem = f
        .path
        .file_stem()
        .map_or_else(|| f.spec.clone(), |s| s.to_string_lossy().into_owned());
    let size = std::fs::metadata(&f.path).map_or(0, |m| m.len());
    let mut e = WorldEntry {
        spec: f.spec.clone(),
        id: format!("local/{stem}"),
        version: 0,
        name: stem.clone(),
        path: Some(f.path.clone()),
        source: f.source,
        size_bytes: size,
        created_unix_s: None,
        status: Status::Unsigned,
        blocked: None,
        summary: None,
        attributions: Vec::new(),
        parts: Vec::new(),
        also: Vec::new(),
    };
    let summary = if f.path.is_file() {
        memo.get(&f.path)
    } else {
        Err("the package file is missing".into())
    };
    let mut broken: Option<String> = None;
    match summary {
        Ok(s) => {
            if !s.name.is_empty() {
                e.name.clone_from(&s.name);
            }
            e.summary = Some(s);
        }
        Err(why) => broken = Some(why),
    }
    match f.manifest {
        None => {}
        Some(Err(why)) => broken = broken.or(Some(why)),
        Some(Ok(m)) => {
            let man = &m.manifest;
            e.id.clone_from(&man.id);
            e.version = man.version;
            e.name.clone_from(&man.name);
            e.created_unix_s = Some(man.created_unix_s);
            e.attributions.clone_from(&man.attribution);
            let file = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            match man.file(file).or(man.package_file()) {
                Some(entry) if f.path.is_file() && entry.size != size => {
                    // Says more than the layout error it usually causes.
                    broken = Some(format!(
                        "{size} bytes, the manifest says {} (damaged or incomplete)",
                        entry.size
                    ));
                }
                None => broken = broken.or(Some("the manifest lists no package file".into())),
                _ => {}
            }
            e.status = match m.verify_signature(&d.trust, d.now_unix_s) {
                Ok(s) => Status::Verified(s.root_name.clone()),
                Err(VerifyError::Unsigned) => Status::Unsigned,
                Err(err) if err.is_trust_failure() => Status::Untrusted(err.to_string()),
                Err(err) => {
                    broken = broken.or(Some(err.to_string()));
                    Status::Untrusted(err.to_string())
                }
            };
        }
    }
    if e.attributions.is_empty() && e.summary.is_some() {
        // Packages are built from OSM (spec §13), with or without a manifest.
        e.attributions.push(Attribution::openstreetmap());
    }
    if let Some(why) = broken {
        e.blocked = Some(why.clone());
        e.status = Status::Broken(why);
        return e;
    }
    let from_cache = e.spec.contains('@') && !Path::new(&e.spec).is_file();
    let trusted = matches!(e.status, Status::Verified(_));
    if !trusted && from_cache && !d.allow_unsigned {
        e.blocked = Some(format!(
            "{} (cached packages must be signed by a trusted key; --allow-unsigned accepts it)",
            match &e.status {
                Status::Untrusted(why) => why.clone(),
                _ => "not signed".into(),
            }
        ));
    } else if !trusted && !from_cache && d.require_signed {
        e.blocked = Some("not signed by a trusted key (--require-signed)".into());
    }
    e
}

/// One entry per manifest id (see the module docs).
fn fold_duplicates(entries: Vec<WorldEntry>) -> Vec<WorldEntry> {
    let mut out: Vec<WorldEntry> = Vec::new();
    for e in entries {
        let local = e.id.starts_with("local/");
        let same = (!local && !e.status.is_broken())
            .then(|| {
                out.iter()
                    .position(|o| o.id == e.id && !o.status.is_broken())
            })
            .flatten();
        let Some(i) = same else {
            out.push(e);
            continue;
        };
        let better =
            e.version > out[i].version || (e.version == out[i].version && e.source < out[i].source);
        if better {
            let old = std::mem::replace(&mut out[i], e);
            out[i].also.push(old.spec);
            out[i].also.extend(old.also);
        } else {
            out[i].also.push(e.spec);
        }
    }
    out
}

/// Neighbours: regions that meet at a border (boxes touching, little
/// shared).
fn neighbours(a: &WorldEntry, b: &WorldEntry) -> bool {
    let region = |e: &WorldEntry| e.summary.as_ref().and_then(|s| s.region.clone());
    let (Some(ra), Some(rb)) = (region(a), region(b)) else {
        return false;
    };
    if a.id == b.id || !ra.boxes_touch(&rb, TOUCH_TOLERANCE_DEG) {
        return false;
    }
    // Boxes that hardly overlap (cuts along a line, like the Black Forest's
    // three packages on 48.12 N and 48.45 N) need no sampling of polygons.
    let bx = |r: &ocw_world::region::PackageRegion| {
        ocw_world::region::PackageRegion::bbox(r.south, r.west, r.north, r.east)
    };
    bx(&ra).shared_fraction(&bx(&rb)) < PART_SHARED_MAX || ra.shared_fraction(&rb) < PART_SHARED_MAX
}

/// Folds neighbouring packages into one world with parts; the given
/// package (else the first by name) leads its world. Sorted: given first,
/// then by name, broken packages last.
fn group(mut entries: Vec<WorldEntry>) -> Vec<WorldEntry> {
    entries.sort_by(|a, b| {
        (
            a.status.is_broken(),
            a.source != Source::Given,
            a.name.to_lowercase(),
        )
            .cmp(&(
                b.status.is_broken(),
                b.source != Source::Given,
                b.name.to_lowercase(),
            ))
            .then(a.spec.cmp(&b.spec))
    });
    let n = entries.len();
    let mut root: Vec<usize> = (0..n).collect();
    fn find(root: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while root[i] != i {
            root[i] = root[root[i]];
            i = root[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            if entries[i].is_rideable()
                && entries[j].is_rideable()
                && neighbours(&entries[i], &entries[j])
            {
                let (a, b) = (find(&mut root, i), find(&mut root, j));
                // The earlier entry (given / first by name) leads.
                root[a.max(b)] = a.min(b);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut root, i);
        groups.entry(r).or_default().push(i);
    }
    let mut slots: Vec<Option<WorldEntry>> = entries.into_iter().map(Some).collect();
    let mut out = Vec::new();
    for (lead, members) in groups {
        let mut e = slots[lead].take().expect("each index once");
        for m in members.into_iter().filter(|&m| m != lead) {
            e.parts.push(slots[m].take().expect("each index once"));
        }
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::WorldPackage;
    use ocw_world::manifest::{Manifest, RootKeyPair, SigningKeyPair};
    use ocw_world::package::{Tile, TileKey};
    use ocw_world::region::PackageRegion;

    const NOW: i64 = 1_800_000_000;

    fn keys() -> (RootKeyPair, SigningKeyPair) {
        let root = RootKeyPair::from_seed("Test root", [5; 32]);
        let cert = root.certify(
            "s",
            SigningKeyPair::public_key_of([6; 32]),
            NOW - 10,
            NOW + 1000,
        );
        (root, SigningKeyPair::new([6; 32], cert).unwrap())
    }

    fn package(name: &str, region: Option<[f64; 4]>, routes: &[&str]) -> Vec<u8> {
        let key = TileKey { x: 0, z: 0 };
        WorldPackage {
            name: name.into(),
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
            routes: routes
                .iter()
                .map(|r| ocw_world::GraphRoute {
                    name: (*r).into(),
                    steps: Vec::new(),
                    is_loop: false,
                })
                .collect(),
            region: region.map(|[s, w, n, e]| PackageRegion::bbox(s, w, n, e)),
            time_zone: Some("Europe/Vaduz".into()),
            bridges: None,
            landuse: None,
            buildings: None,
            look: Default::default(),
        }
        .to_bytes()
    }

    /// Writes `dir/FILE` and, with `id`, its signed (or unsigned) sidecar
    /// manifest.
    fn write(dir: &Path, file: &str, bytes: &[u8], id: Option<(&str, u64, bool)>) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(file);
        std::fs::write(&p, bytes).unwrap();
        if let Some((id, version, signed)) = id {
            let mut m = Manifest::new(
                id,
                &format!("{id} region"),
                version,
                NOW - 5,
                "test",
                vec![Attribution::openstreetmap(), Attribution::srtm()],
            )
            .unwrap();
            m.add_file(file, bytes).unwrap();
            let s = if signed {
                SignedManifest::signed(&m, &keys().1)
            } else {
                SignedManifest::unsigned(&m)
            };
            std::fs::write(dir.join(format!("{file}.manifest.json")), s.to_json()).unwrap();
        }
        p
    }

    /// Installs `bytes` into the cache as `id@version`.
    fn install(cache: &Path, id: &str, version: u64, name: &str, bytes: &[u8]) {
        let dir = cache.join(id).join(version.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = Manifest::new(id, name, version, NOW - 5, "test", vec![]).unwrap();
        m.add_file("package.ocwpkg", bytes).unwrap();
        std::fs::write(dir.join("package.ocwpkg"), bytes).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            SignedManifest::signed(&m, &keys().1).to_json(),
        )
        .unwrap();
    }

    fn discovery(root: &Path, given: Option<&Path>, dirs: &[PathBuf]) -> Discovery {
        Discovery {
            cache: WorldCache::new(root.join("cache")),
            given: given.map(|p| p.display().to_string()),
            dirs: dirs.to_vec(),
            trust: TrustStore::new([keys().0.public()]),
            now_unix_s: NOW,
            allow_unsigned: false,
            require_signed: false,
        }
    }

    const LI: [f64; 4] = [47.05, 9.47, 47.27, 9.64];
    const CH_EAST: [f64; 4] = [46.8, 9.64, 47.2, 10.0];
    const BF: [f64; 4] = [47.6, 7.6, 48.1, 8.3];

    #[test]
    fn worlds_come_from_the_cache_the_given_package_and_its_siblings() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let kiosk = root.join("kiosk");
        let li = write(
            &kiosk,
            "liechtenstein.ocwpkg",
            &package("Liechtenstein", Some(LI), &["Vaduz - Malbun", "Rhine"]),
            Some(("europe/liechtenstein", 3, true)),
        );
        write(
            &kiosk,
            "blackforest.ocwpkg",
            &package("Black Forest", Some(BF), &["Schauinsland"]),
            Some(("europe/blackforest", 1, true)),
        );
        // An unsigned test build of the same area: its own world.
        write(
            &kiosk,
            "liechtenstein-freeride.ocwpkg",
            &package("Liechtenstein", Some(LI), &[]),
            None,
        );
        install(
            &root.join("cache"),
            "europe/alps",
            2,
            "Alps",
            &package("Alps", None, &[]),
        );
        // The kiosk directory again as a search directory: no duplicates.
        let d = discovery(root, Some(&li), &[kiosk.clone(), root.join("missing")]);
        let w = discover(&d, &mut SummaryMemo::default());
        let names: Vec<(&str, Source)> = w.iter().map(|e| (e.name.as_str(), e.source)).collect();
        assert_eq!(
            names,
            [
                ("europe/liechtenstein region", Source::Given),
                ("Alps", Source::Cache),
                ("europe/blackforest region", Source::Sibling),
                ("Liechtenstein", Source::Sibling),
            ]
        );
        let l = &w[0];
        assert_eq!((l.id.as_str(), l.version), ("europe/liechtenstein", 3));
        assert_eq!(l.status, Status::Verified("Test root".into()));
        assert_eq!(l.routes(), 2);
        assert_eq!(l.first_route(), Some("Vaduz - Malbun"));
        assert_eq!(l.attributions.len(), 2);
        assert_eq!(l.created_unix_s, Some(NOW - 5));
        assert!(l.is_rideable() && l.parts.is_empty());
        assert!(l.detail().starts_with("v3, "), "{}", l.detail());
        assert!(l.detail().ends_with(", verified"), "{}", l.detail());
        assert!(l.area_km2().unwrap() > 100.0);
        assert_eq!(w[1].spec, "europe/alps@2");
        let free = &w[3];
        assert_eq!(free.status, Status::Unsigned);
        assert_eq!(free.id, "local/liechtenstein-freeride");
        assert!(free.is_rideable(), "a path may be unsigned");
        assert_eq!(free.attributions, [Attribution::openstreetmap()]);
        assert!(free.detail().starts_with("local, "));
        // --require-signed: the unsigned file is listed but blocked.
        let strict = Discovery {
            require_signed: true,
            ..d.clone()
        };
        let w = discover(&strict, &mut SummaryMemo::default());
        let free = w
            .iter()
            .find(|e| e.id == "local/liechtenstein-freeride")
            .unwrap();
        assert!(
            free.blocked
                .as_deref()
                .unwrap()
                .contains("--require-signed")
        );
    }

    #[test]
    fn duplicates_fold_into_the_newest_version() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let bytes = package("Liechtenstein", Some(LI), &["A"]);
        install(
            &root.join("cache"),
            "europe/liechtenstein",
            2,
            "Liechtenstein",
            &bytes,
        );
        install(
            &root.join("cache"),
            "europe/liechtenstein",
            3,
            "Liechtenstein",
            &bytes,
        );
        let file = write(
            &root.join("files"),
            "li.ocwpkg",
            &bytes,
            Some(("europe/liechtenstein", 3, true)),
        );
        let d = discovery(root, None, &[root.join("files")]);
        let w = discover(&d, &mut SummaryMemo::default());
        assert_eq!(w.len(), 1, "{w:#?}");
        // Version 3 from the cache wins over the same version as a file.
        assert_eq!(w[0].spec, "europe/liechtenstein@3");
        assert_eq!(w[0].also.len(), 2, "{:?}", w[0].also);
        // Given with --world, the file wins the tie.
        let d = discovery(root, Some(&file), &[]);
        let w = discover(&d, &mut SummaryMemo::default());
        assert_eq!(w.len(), 1);
        assert_eq!(
            (w[0].source, w[0].spec.clone()),
            (Source::Given, file.display().to_string())
        );
        // The given cache id: listed as given.
        let d = Discovery {
            given: Some("europe/liechtenstein".into()),
            ..discovery(root, None, &[])
        };
        let w = discover(&d, &mut SummaryMemo::default());
        assert_eq!(
            (w[0].source, w[0].spec.as_str()),
            (Source::Given, "europe/liechtenstein@3")
        );
    }

    #[test]
    fn broken_and_unsigned_packages_are_listed_with_a_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let dir = root.join("files");
        write(&dir, "zip.ocwpkg", b"PK\x03\x04 not a package", None);
        let good = package("Good", Some(LI), &[]);
        write(&dir, "truncated.ocwpkg", &good[..good.len() - 7], None);
        // A manifest that does not match the file's size.
        let p = write(&dir, "short.ocwpkg", &good, Some(("test/short", 1, true)));
        std::fs::write(&p, &good[..good.len() - 1]).unwrap();
        // A manifest that is not JSON.
        write(&dir, "badman.ocwpkg", &good, None);
        std::fs::write(dir.join("badman.ocwpkg.manifest.json"), "{ nope").unwrap();
        // Signed by a root nobody trusts.
        let other = RootKeyPair::from_seed("Other root", [9; 32]);
        let cert = other.certify(
            "o",
            SigningKeyPair::public_key_of([8; 32]),
            NOW - 10,
            NOW + 1000,
        );
        let mut m = Manifest::new("test/stranger", "Stranger", 1, NOW - 5, "test", vec![]).unwrap();
        m.add_file("stranger.ocwpkg", &good).unwrap();
        std::fs::write(dir.join("stranger.ocwpkg"), &good).unwrap();
        std::fs::write(
            dir.join("stranger.ocwpkg.manifest.json"),
            SignedManifest::signed(&m, &SigningKeyPair::new([8; 32], cert).unwrap()).to_json(),
        )
        .unwrap();
        // An unsigned package in the cache.
        let cdir = root.join("cache/test/unsigned/1");
        std::fs::create_dir_all(&cdir).unwrap();
        let mut m = Manifest::new("test/unsigned", "Unsigned", 1, NOW - 5, "test", vec![]).unwrap();
        m.add_file("package.ocwpkg", &good).unwrap();
        std::fs::write(cdir.join("package.ocwpkg"), &good).unwrap();
        std::fs::write(
            cdir.join("manifest.json"),
            SignedManifest::unsigned(&m).to_json(),
        )
        .unwrap();
        let d = discovery(root, None, std::slice::from_ref(&dir));
        let w = discover(&d, &mut SummaryMemo::default());
        let by = |id: &str| {
            w.iter()
                .find(|e| e.id == id)
                .unwrap_or_else(|| panic!("{id} in {w:#?}"))
        };
        let reason = |id: &str| by(id).blocked.clone().unwrap_or_default();
        assert!(reason("local/zip").contains("not an OCW world package"));
        assert!(reason("local/truncated").contains("truncated"));
        assert!(
            reason("test/short").contains("damaged or incomplete"),
            "{}",
            reason("test/short")
        );
        assert!(
            reason("local/badman").contains("manifest"),
            "{}",
            reason("local/badman")
        );
        for id in ["local/zip", "local/truncated", "test/short", "local/badman"] {
            assert!(by(id).status.is_broken(), "{id}");
        }
        // Untrusted, but a path: rideable with a warning (as load_world).
        let s = by("test/stranger");
        assert!(matches!(s.status, Status::Untrusted(_)), "{:?}", s.status);
        assert!(s.is_rideable());
        // Unsigned in the cache: blocked unless --allow-unsigned.
        assert!(reason("test/unsigned").contains("--allow-unsigned"));
        let lax = Discovery {
            allow_unsigned: true,
            ..d.clone()
        };
        let w2 = discover(&lax, &mut SummaryMemo::default());
        assert!(
            w2.iter()
                .find(|e| e.id == "test/unsigned")
                .unwrap()
                .is_rideable()
        );
        // Broken ones come last.
        let broken: Vec<bool> = w.iter().map(|e| e.status.is_broken()).collect();
        let first_broken = broken.iter().position(|b| *b).unwrap();
        assert!(broken[first_broken..].iter().all(|b| *b), "{broken:?}");
    }

    #[test]
    fn neighbouring_packages_are_one_world_with_parts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let dir = root.join("files");
        let li = write(
            &dir,
            "li.ocwpkg",
            &package("Liechtenstein", Some(LI), &[]),
            Some(("europe/liechtenstein", 1, true)),
        );
        write(
            &dir,
            "east.ocwpkg",
            &package("Graubuenden", Some(CH_EAST), &[]),
            Some(("europe/graubuenden", 1, true)),
        );
        write(
            &dir,
            "bf.ocwpkg",
            &package("Black Forest", Some(BF), &[]),
            Some(("europe/blackforest", 1, true)),
        );
        let d = discovery(root, Some(&li), &[]);
        let w = discover(&d, &mut SummaryMemo::default());
        assert_eq!(w.len(), 2, "{w:#?}");
        assert_eq!(w[0].id, "europe/liechtenstein", "the given package leads");
        let parts: Vec<&str> = w[0].parts.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(parts, ["europe/graubuenden"]);
        assert_eq!(w[0].members().count(), 2);
        assert!(w[1].parts.is_empty());
    }

    /// The Black Forest in three packages cut on 48.12 N and 48.45 N (spec
    /// v2.36): one world, the south leading by name, middle and north its
    /// parts (the north touches only the middle: a chain).
    #[test]
    fn the_three_black_forest_packages_are_one_world() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("kiosk");
        let li = write(
            &dir,
            "liechtenstein.ocwpkg",
            &package("Liechtenstein", Some(LI), &["Vaduz - Malbun"]),
            Some(("europe/liechtenstein", 8, true)),
        );
        for (file, id, box_) in [
            (
                "blackforest.ocwpkg",
                "europe/blackforest",
                [47.55, 7.5, 48.12, 8.4],
            ),
            (
                "blackforest-middle.ocwpkg",
                "europe/blackforest-middle",
                [48.12, 7.7, 48.45, 8.5],
            ),
            (
                "blackforest-north.ocwpkg",
                "europe/blackforest-north",
                [48.45, 7.9, 48.8, 8.5],
            ),
        ] {
            write(
                &dir,
                file,
                &package(id, Some(box_), &["r"]),
                Some((id, 1, true)),
            );
        }
        let d = discovery(tmp.path(), Some(&li), &[]);
        let w = discover(&d, &mut SummaryMemo::default());
        let ids: Vec<&str> = w.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            ["europe/liechtenstein", "europe/blackforest"],
            "{w:#?}"
        );
        let parts: Vec<&str> = w[1].parts.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            parts,
            ["europe/blackforest-middle", "europe/blackforest-north"]
        );
    }

    #[test]
    fn summaries_are_read_again_only_when_a_file_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(tmp.path(), "a.ocwpkg", &package("A", None, &["r"]), None);
        let mut memo = SummaryMemo::default();
        assert_eq!(memo.get(&p).unwrap().routes.len(), 1);
        // Same size and time: the memo answers (even if the bytes changed).
        assert_eq!(memo.0.len(), 1);
        std::fs::write(&p, package("A", None, &["r", "s"])).unwrap();
        assert_eq!(memo.get(&p).unwrap().routes.len(), 2, "the size changed");
        assert!(megabytes(30_174_974) == "30 MB" && megabytes(2_500_000) == "2.5 MB");
    }
}
