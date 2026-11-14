//! Distribution subcommands (spec §4.7, §12): keys, manifests, signing,
//! verification, the published directory layout and rebuild diffs.

use std::path::{Path, PathBuf};

use ocw_world::manifest::{
    Attribution, Manifest, RootKeyPair, SignedManifest, SigningKeyPair, UnsignedPolicy,
    apply_policy, sha256,
};
use ocw_worldstore::keys::{
    default_key_dir, random_seed, read_root_secret, read_signing_key, write_new,
};
use ocw_worldstore::trust::TrustConfig;
use ocw_worldstore::{manifest_path_for, now_unix_s};

pub const GENERATOR: &str = concat!("world-builder ", env!("CARGO_PKG_VERSION"));
const DAY_S: i64 = 86_400;

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// `<package>.manifest.json`.
pub fn sidecar(package: &Path) -> PathBuf {
    let mut s = package.as_os_str().to_owned();
    s.push(".manifest.json");
    PathBuf::from(s)
}

/// Lower-case id from a region name ("Liechtenstein" → "liechtenstein").
pub fn id_from_name(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "world".into() } else { s }
}

/// YYYY-MM-DD (UTC) of a Unix time.
pub fn date(unix_s: i64) -> String {
    // Days to civil date (H. Hinnant's algorithm).
    let z = unix_s.div_euclid(DAY_S) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The DEM attribution: SRTM, a known high-resolution DTM (spec §4.3,
/// Phase 8A licences) by its short name, or a custom notice.
pub fn dem_attribution(notice: Option<&str>) -> Attribution {
    let known = |source: &str, licence: &str, notice: &str, url: &str| Attribution {
        source: source.into(),
        licence: licence.into(),
        notice: notice.into(),
        url: Some(url.into()),
    };
    match notice {
        None | Some("srtm") => Attribution::srtm(),
        Some("lgl-dgm1") => known(
            "LGL DGM1 Baden-Württemberg",
            "dl-de/by-2-0",
            "Datenquelle: LGL, www.lgl-bw.de, dl-de/by-2-0 (Digitales Geländemodell DGM1, \
             averaged to 2 m and adapted to the roads)",
            "https://www.govdata.de/dl-de/by-2-0",
        ),
        Some("swissalti3d") => known(
            "swissALTI3D",
            "swisstopo OGD (open use, source must be named)",
            "© swisstopo: swissALTI3D (2 m, adapted to the roads)",
            "https://www.swisstopo.admin.ch/en/terms-of-use-free-geodata-and-geoservices",
        ),
        Some("ign-mdt05") => known(
            "IGN MDT05 (Spain, Canary Islands)",
            "CC BY 4.0",
            "MDT05-cob1 2008-2015 CC-BY 4.0 scne.es (Instituto Geográfico Nacional de España; \
             derived work: moved onto the 5 m lattice and adapted to the roads)",
            "https://creativecommons.org/licenses/by/4.0/",
        ),
        Some(n) => Attribution {
            source: "Elevation model".into(),
            licence: "see notice".into(),
            notice: n.into(),
            url: None,
        },
    }
}

/// The attributions of the imagery sources in a package's palette section
/// (spec §13, Phase 8A): one per source, its notice as given by the fetch
/// script (licence, holder, year).
pub fn imagery_attribution(pkg: &ocw_world::WorldPackage) -> Vec<Attribution> {
    pkg.look
        .sources()
        .iter()
        .map(|s| Attribution {
            source: s.name.clone(),
            licence: s.licence.clone(),
            notice: s.notice.clone(),
            url: (!s.url.is_empty()).then(|| s.url.clone()),
        })
        .collect()
}

/// What to put into a new manifest.
pub struct ManifestArgs {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<u64>,
    /// Elevation sources (empty: keep the old manifest's, else SRTM).
    pub dem_attribution: Vec<String>,
}

/// Writes (or rewrites) `<package>.manifest.json` for `package`, signed
/// with `key` if given. Fields not given are taken from an existing
/// manifest; a new build of the same id gets the next version.
pub fn write_manifest(
    package: &Path,
    args: &ManifestArgs,
    key: Option<&SigningKeyPair>,
) -> SignedManifest {
    let bytes =
        std::fs::read(package).unwrap_or_else(|e| fail(format!("{}: {e}", package.display())));
    let pkg = ocw_world::WorldPackage::from_bytes(&bytes)
        .unwrap_or_else(|e| fail(format!("{}: {e}", package.display())));
    let side = sidecar(package);
    let old = std::fs::read_to_string(&side)
        .ok()
        .and_then(|t| SignedManifest::parse(&t).ok())
        .map(|s| s.manifest);
    let name = args
        .name
        .clone()
        .or_else(|| old.as_ref().map(|m| m.name.clone()))
        .unwrap_or_else(|| pkg.name.clone());
    let id = args
        .id
        .clone()
        .or_else(|| old.as_ref().map(|m| m.id.clone()))
        .unwrap_or_else(|| id_from_name(&name));
    let file = package
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| fail("package path has no file name"));
    let version = args.version.unwrap_or_else(|| match &old {
        // Same content: keep the version (re-signing); new content: next one.
        Some(m) if m.id == id && m.file(file).is_some_and(|f| f.sha256 == sha256(&bytes)) => {
            m.version
        }
        Some(m) if m.id == id => m.version + 1,
        _ => 1,
    });
    let mut attribution = vec![Attribution::openstreetmap()];
    match (args.dem_attribution.as_slice(), &old) {
        ([], Some(m)) => attribution.extend(
            m.attribution
                .iter()
                .filter(|a| a.source != "OpenStreetMap")
                .cloned(),
        ),
        ([], None) => attribution.push(dem_attribution(None)),
        (dems, _) => attribution.extend(dems.iter().map(|d| dem_attribution(Some(d)))),
    }
    // The imagery the package's look was measured in (section PALT, spec
    // §14 Phase 8 T3): its notices go into the credits and LICENSE-DATA.md.
    for a in imagery_attribution(&pkg) {
        if !attribution.iter().any(|b| b.source == a.source) {
            attribution.push(a);
        }
    }
    let mut m = Manifest::new(&id, &name, version, now_unix_s(), GENERATOR, attribution)
        .unwrap_or_else(|e| fail(e));
    m.add_file(file, &bytes).unwrap_or_else(|e| fail(e));
    let signed = match key {
        Some(k) => SignedManifest::signed(&m, k),
        None => SignedManifest::unsigned(&m),
    };
    std::fs::write(&side, signed.to_json())
        .unwrap_or_else(|e| fail(format!("{}: {e}", side.display())));
    println!(
        "manifest: {} ({}@{}, {}, {} chunks)",
        side.display(),
        m.id,
        m.version,
        if key.is_some() { "signed" } else { "unsigned" },
        m.files[0].chunks.len()
    );
    signed
}

pub fn keys_init_root(name: &str, dir: Option<PathBuf>) {
    let dir = dir.unwrap_or_else(default_key_dir);
    let root = RootKeyPair::from_seed(name, random_seed().unwrap_or_else(|e| fail(e)));
    let (secret, public) = (dir.join("root.key"), dir.join("root.pub.json"));
    write_new(&secret, &root.to_json(), true).unwrap_or_else(|e| fail(e));
    write_new(&public, &root.public().to_json(), false).unwrap_or_else(|e| fail(e));
    println!(
        "root key {} ({}):\n  secret: {}  <- keep OFFLINE, never commit\n  public: {}  <- clients trust this",
        root.public().key_id,
        name,
        secret.display(),
        public.display()
    );
}

pub fn keys_new_signing(root: Option<PathBuf>, name: &str, days: u32, out: Option<PathBuf>) {
    let dir = default_key_dir();
    let root_path = root.unwrap_or_else(|| dir.join("root.key"));
    let root = read_root_secret(&root_path).unwrap_or_else(|e| fail(e));
    let seed = random_seed().unwrap_or_else(|e| fail(e));
    let now = now_unix_s();
    // Valid from an hour ago (clock skew between machines).
    let cert = root.certify(
        name,
        SigningKeyPair::public_key_of(seed),
        now - 3600,
        now + i64::from(days) * DAY_S,
    );
    let key = SigningKeyPair::new(seed, cert).unwrap_or_else(|e| fail(e));
    let out = out.unwrap_or_else(|| {
        root_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{}.key", id_from_name(name)))
    });
    write_new(&out, &key.to_json(), true).unwrap_or_else(|e| fail(e));
    let c = key.claims();
    println!(
        "signing key {} ({name}), signed by root {}, valid {} .. {}:\n  {}",
        c.key_id,
        c.root_key_id,
        date(c.valid_from_unix_s),
        date(c.valid_until_unix_s),
        out.display()
    );
}

pub fn sign(package: &Path, key: &Path, args: &ManifestArgs) {
    let key = read_signing_key(key).unwrap_or_else(|e| fail(e));
    let c = key.claims();
    let now = now_unix_s();
    if now > c.valid_until_unix_s || now < c.valid_from_unix_s {
        fail(format!(
            "signing key {} is not valid now (valid {} .. {}); create a new one with `world-builder keys new-signing`",
            c.key_id,
            date(c.valid_from_unix_s),
            date(c.valid_until_unix_s)
        ));
    }
    write_manifest(package, args, Some(&key));
}

/// Verifies a package and its manifest; exits 1 unless it is intact and
/// signed by a trusted root (`allow_unsigned`: intact is enough).
pub fn verify(package: &Path, trust_roots: Vec<PathBuf>, dev_root: bool, allow_unsigned: bool) {
    let trust = TrustConfig {
        dev_roots: dev_root,
        root_files: trust_roots,
        ..TrustConfig::for_build()
    }
    .trust_store()
    .unwrap_or_else(|e| fail(e));
    let Some(mp) = manifest_path_for(package) else {
        fail(format!(
            "{}: no manifest ({} missing); create one with `world-builder sign`",
            package.display(),
            sidecar(package).display()
        ));
    };
    let text =
        std::fs::read_to_string(&mp).unwrap_or_else(|e| fail(format!("{}: {e}", mp.display())));
    let signed =
        SignedManifest::parse(&text).unwrap_or_else(|e| fail(format!("{}: {e}", mp.display())));
    let m = &signed.manifest;
    println!(
        "package {}@{} \"{}\" ({}), created {} by {}",
        m.id,
        m.version,
        m.name,
        m.package_format,
        date(m.created_unix_s),
        m.generator
    );
    let dir = mp.parent().unwrap_or(Path::new("."));
    let mut ok = true;
    for f in &m.files {
        let path = dir.join(&f.path);
        let result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| m.verify_file(&f.path, &b).map_err(|e| e.to_string()));
        ok &= result.is_ok();
        println!(
            "  {} {:.1} MB, {} chunks, sha256 {}: {}",
            f.path,
            f.size as f64 / 1e6,
            f.chunks.len(),
            f.sha256,
            result.err().unwrap_or_else(|| "OK".into())
        );
    }
    for a in &m.attribution {
        println!("  attribution: {} ({})", a.notice, a.licence);
    }
    let policy = if allow_unsigned {
        UnsignedPolicy::Warn
    } else {
        UnsignedPolicy::Refuse
    };
    match apply_policy(signed.verify_signature(&trust, now_unix_s()), policy) {
        Ok(t) => println!("  signature: {t}"),
        Err(e) => {
            ok = false;
            println!("  signature: REFUSED: {e}");
            if let Some(k) = signed.signing_key() {
                println!(
                    "  (signing key {} \"{}\" of root {})",
                    k.key_id, k.name, k.root_key_id
                );
            }
        }
    }
    if !ok {
        std::process::exit(1);
    }
}

/// Copies a verified package into `<out>/<id>/<version>/` (manifest.json
/// and the files), the layout clients download from.
pub fn publish(package: &Path, out: &Path) {
    let Some(mp) = manifest_path_for(package) else {
        fail(format!(
            "{}: no manifest; run `world-builder sign` first",
            package.display()
        ));
    };
    let text = std::fs::read_to_string(&mp).unwrap_or_else(|e| fail(e));
    let signed = SignedManifest::parse(&text).unwrap_or_else(|e| fail(e));
    let m = &signed.manifest;
    if !signed.is_signed() {
        eprintln!("warning: publishing an unsigned package (clients refuse it by default)");
    }
    let dest = out.join(&m.id).join(m.version.to_string());
    std::fs::create_dir_all(&dest).unwrap_or_else(|e| fail(format!("{}: {e}", dest.display())));
    let src = mp.parent().unwrap_or(Path::new("."));
    for f in &m.files {
        let bytes =
            std::fs::read(src.join(&f.path)).unwrap_or_else(|e| fail(format!("{}: {e}", f.path)));
        m.verify_file(&f.path, &bytes).unwrap_or_else(|e| fail(e));
        std::fs::write(dest.join(&f.path), &bytes).unwrap_or_else(|e| fail(e));
    }
    std::fs::write(dest.join(ocw_worldstore::cache::MANIFEST_FILE), &text)
        .unwrap_or_else(|e| fail(e));
    // `<id>/latest`: the newest version, for clients that name the world by
    // id only (the browser app's `?world=ID`, spec §14 Phase 10 B2). An
    // older version published later leaves it alone.
    let latest = out.join(&m.id).join(ocw_worldstore::stream::LATEST_FILE);
    let newer = std::fs::read_to_string(&latest)
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .is_none_or(|v| m.version >= v);
    if newer {
        std::fs::write(&latest, format!("{}\n", m.version)).unwrap_or_else(|e| fail(e));
    }
    println!(
        "published {}@{} to {}\n  clients: ocw-desktop --fetch-world <base URL>/{}/{}/manifest.json",
        m.id,
        m.version,
        dest.display(),
        m.id,
        m.version
    );
}

/// Compares the road graphs of two builds by stable edge ID (spec §4.4).
pub fn diff(old: &Path, new: &Path) {
    let read = |p: &Path| {
        let bytes = std::fs::read(p).unwrap_or_else(|e| fail(format!("{}: {e}", p.display())));
        ocw_world::WorldPackage::from_bytes(&bytes)
            .unwrap_or_else(|e| fail(format!("{}: {e}", p.display())))
            .graph
            .unwrap_or_else(|| fail(format!("{}: no road graph", p.display())))
    };
    let (a, b) = (read(old), read(new));
    let d = ocw_worldgen::diff::GraphDiff::between(&a, &b);
    println!("{}", d.summary());
    let describe = |g: &ocw_world::graph::RoadGraph, id| {
        g.edges
            .iter()
            .find(|e| e.id == id)
            .map(|e| {
                format!(
                    "{:?} {} ({:.0} m, way {})",
                    e.class,
                    e.name.as_deref().unwrap_or("unnamed"),
                    e.length_m(),
                    e.osm_way
                )
            })
            .unwrap_or_default()
    };
    for (id, off) in d.moved.iter().take(20) {
        println!("  moved {:>20}  {:.1} m  {}", id.0, off, describe(&b, *id));
    }
    for id in d.removed.iter().take(20) {
        println!("  removed {:>18}  {}", id.0, describe(&a, *id));
    }
    for id in d.added.iter().take(20) {
        println!("  added {:>20}  {}", id.0, describe(&b, *id));
    }
}

/// After a command rewrote `package` (routes added or removed, `gpx`,
/// `route --world`, `routes`): the manifest beside it, if any, lists the old
/// SHA-256, so every client would refuse the package as damaged (spec §4.7).
/// Writes it again for the new content (same id, name and attribution, next
/// version). A signature cannot be renewed without the key: the new
/// manifest is unsigned and a note says to sign it again. Returns whether
/// a manifest was rewritten.
pub fn refresh_manifest(package: &Path) -> bool {
    let side = sidecar(package);
    let Some(old) = std::fs::read_to_string(&side)
        .ok()
        .and_then(|t| SignedManifest::parse(&t).ok())
    else {
        return false;
    };
    write_manifest(
        package,
        &ManifestArgs {
            id: None,
            name: None,
            version: None,
            dem_attribution: Vec::new(),
        },
        None,
    );
    if old.is_signed() {
        println!(
            "note: the manifest was signed; sign it again: world-builder sign --key KEY {}",
            package.display()
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `world-builder route --world` (and `gpx`, `routes`) wrote
    /// the package but left the manifest with the old hash, so the client
    /// refused the package with the new routes as damaged.
    #[test]
    fn a_rewritten_package_gets_a_matching_manifest() {
        let dir = std::env::temp_dir().join(format!("ocw-wb-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.ocwpkg");
        let mut pkg = ocw_world::WorldPackage {
            name: "Test".into(),
            origin: (47.9, 7.9),
            tile_size_m: 1000.0,
            samples: 3,
            tiles: Default::default(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![],
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
        };
        std::fs::write(&path, pkg.to_bytes()).unwrap();
        // No manifest yet: nothing to refresh.
        assert!(!refresh_manifest(&path));
        let args = ManifestArgs {
            id: Some("test/refresh".into()),
            name: None,
            version: None,
            dem_attribution: Vec::new(),
        };
        write_manifest(&path, &args, None);
        // The package changes (a route is added) after the manifest was written.
        pkg.put_route(ocw_world::GraphRoute {
            name: "R".into(),
            is_loop: false,
            steps: vec![],
        });
        let bytes = pkg.to_bytes();
        std::fs::write(&path, &bytes).unwrap();
        let read = |p: &Path| {
            SignedManifest::parse(&std::fs::read_to_string(sidecar(p)).unwrap())
                .unwrap()
                .manifest
        };
        assert_ne!(read(&path).file("p.ocwpkg").unwrap().sha256, sha256(&bytes));
        assert!(refresh_manifest(&path));
        let m = read(&path);
        assert_eq!(m.file("p.ocwpkg").unwrap().sha256, sha256(&bytes));
        assert_eq!((m.id.as_str(), m.version), ("test/refresh", 2));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `publish` writes `<id>/<version>/` and `<id>/latest` (the browser
    /// app's `?world=ID`, spec §14 Phase 10 B2); an older version
    /// published later does not lower `latest`.
    #[test]
    fn publish_writes_the_version_and_latest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.ocwpkg");
        let pkg = ocw_world::WorldPackage {
            name: "Test".into(),
            origin: (47.9, 7.9),
            tile_size_m: 1000.0,
            samples: 3,
            tiles: Default::default(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![],
            region: None,
            time_zone: Some("Europe/Vaduz".into()),
            look: Default::default(),
            obstacles: None,
        };
        std::fs::write(&path, pkg.to_bytes()).unwrap();
        let out = dir.path().join("worlds");
        for version in [4, 3] {
            let args = ManifestArgs {
                id: Some("test/pub".into()),
                name: None,
                version: Some(version),
                dem_attribution: Vec::new(),
            };
            write_manifest(&path, &args, None);
            publish(&path, &out);
            let d = out.join("test/pub").join(version.to_string());
            assert!(d.join("manifest.json").is_file());
            assert!(d.join("p.ocwpkg").is_file());
        }
        assert_eq!(
            std::fs::read_to_string(out.join("test/pub/latest")).unwrap(),
            "4\n"
        );
    }

    /// Spec §13, Phase 8A: every imagery source of a calibrated package is
    /// credited in its manifest (and so in LICENSE-DATA.md and on the
    /// credits screen), once, next to OSM and the elevation model.
    #[test]
    fn imagery_sources_are_credited_in_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.ocwpkg");
        let source =
            |id: &str, name: &str, licence: &str, notice: &str| ocw_world::look::ImagerySource {
                id: id.into(),
                name: name.into(),
                licence: licence.into(),
                notice: notice.into(),
                url: String::new(),
                date: "2025".into(),
                resolution_m: 0.5,
            };
        let pkg = ocw_world::WorldPackage {
            obstacles: None,
            name: "Test".into(),
            origin: (47.9, 7.9),
            tile_size_m: 1000.0,
            samples: 3,
            tiles: Default::default(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![],
            region: None,
            time_zone: None,
            look: ocw_world::look::RegionLook {
                palette: Some(ocw_world::look::Palette {
                    sources: vec![
                        source(
                            "lgl-dop20",
                            "LGL DOP20",
                            "dl-de/by-2-0",
                            "Datenquelle: LGL, www.lgl-bw.de, dl-de/by-2-0",
                        ),
                        source(
                            "sentinel-2",
                            "Copernicus Sentinel-2",
                            "Copernicus Sentinel data licence",
                            "Contains modified Copernicus Sentinel data 2023-2025",
                        ),
                    ],
                    entries: vec![],
                }),
                ..Default::default()
            },
        };
        std::fs::write(&path, pkg.to_bytes()).unwrap();
        let args = ManifestArgs {
            id: Some("test/imagery".into()),
            name: None,
            version: None,
            dem_attribution: vec!["lgl-dgm1".into()],
        };
        let m = write_manifest(&path, &args, None).manifest;
        let sources: Vec<&str> = m.attribution.iter().map(|a| a.source.as_str()).collect();
        assert_eq!(
            sources,
            [
                "OpenStreetMap",
                "LGL DGM1 Baden-Württemberg",
                "LGL DOP20",
                "Copernicus Sentinel-2"
            ]
        );
        assert_eq!(m.attribution[3].licence, "Copernicus Sentinel data licence");
        // A rewrite keeps them once.
        assert!(refresh_manifest(&path));
        let again = SignedManifest::parse(&std::fs::read_to_string(sidecar(&path)).unwrap())
            .unwrap()
            .manifest;
        assert_eq!(again.attribution, m.attribution);
    }

    #[test]
    fn helpers() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_700_000_000), "2023-11-14");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(id_from_name("Liechtenstein"), "liechtenstein");
        assert_eq!(id_from_name("Vaduz – Malbun"), "vaduz---malbun");
        assert_eq!(id_from_name("!!"), "world");
        assert_eq!(
            sidecar(Path::new("a/b.ocwpkg")),
            Path::new("a/b.ocwpkg.manifest.json")
        );
        assert_eq!(dem_attribution(None).source, "SRTM elevation data");
        assert_eq!(dem_attribution(Some("© X")).notice, "© X");
        assert_eq!(dem_attribution(Some("srtm")).source, "SRTM elevation data");
        let lgl = dem_attribution(Some("lgl-dgm1"));
        assert_eq!(lgl.licence, "dl-de/by-2-0");
        assert!(
            lgl.notice
                .starts_with("Datenquelle: LGL, www.lgl-bw.de, dl-de/by-2-0")
        );
        let ign = dem_attribution(Some("ign-mdt05"));
        assert_eq!(ign.licence, "CC BY 4.0");
        assert!(
            ign.notice
                .starts_with("MDT05-cob1 2008-2015 CC-BY 4.0 scne.es")
        );
        assert!(
            dem_attribution(Some("swissalti3d"))
                .notice
                .contains("swisstopo")
        );
    }
}
