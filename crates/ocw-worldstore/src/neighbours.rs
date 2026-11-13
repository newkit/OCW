//! Installed neighbours of a world package (spec §2.2.2 region border,
//! §4.7): which other packages continue the roads across its border.
//!
//! **Lookup is geographic**, no neighbour list in the package: a
//! manifest carries the region box of its package (`bounds`), so the
//! candidates are the installed packages whose box touches the home
//! package's region – the newest version of every other id in the cache,
//! and the packages next to a package loaded by path (local builds in one
//! directory). Each candidate is then loaded and **verified like the home
//! package** (same trust policy for cache ids and paths) and added to the
//! [`WorldSet`] only if a road crosses the border to it
//! ([`WorldSet::add_neighbour`]). Nothing is downloaded (spec §4.2).
//!
//! **Chains of packages** (v2.30): the search goes on from every joined
//! neighbour, so a region cut into a row of packages (the Black Forest:
//! south – middle – north) rides as one world from any of them, up to
//! [`MAX_MEMBERS`]. Neighbours are loaded as skeletons (graph, routes,
//! region; [`load_world_skeleton`]) and joined without their tiles, which
//! the renderer reads when the rider comes near (spec §15 memory budget).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use ocw_world::manifest::{PackageIdentity, SignedManifest};
use ocw_world::region::PackageRegion;
use ocw_world::{WorldPackage, WorldSet};

use crate::{LoadOptions, LoadedWorld, load_world, load_world_skeleton, manifest_path_for};

/// Boxes this close (degrees, about 1 km) count as touching; the road
/// links decide.
pub const TOUCH_TOLERANCE_DEG: f64 = 0.01;

/// At most this many packages in one world (the home package included).
pub const MAX_MEMBERS: usize = 8;

/// A package that may be a neighbour.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// What to load it with: `id@version` (cache) or a path.
    pub spec: String,
    pub id: String,
}

fn home_region(home: &LoadedWorld) -> Option<PackageRegion> {
    home.package
        .region
        .clone()
        .or_else(|| home.manifest.as_ref()?.bounds.map(|b| b.region()))
}

/// Installed packages whose region box touches the home package's.
pub fn candidates(home: &LoadedWorld, opts: &LoadOptions) -> Vec<Candidate> {
    let Some(region) = home_region(home) else {
        return Vec::new();
    };
    candidates_near(home, &[region], opts)
}

/// Installed packages (other than the home package) whose region box
/// touches one of `regions`; the folder of the home package is searched.
fn candidates_near(
    home: &LoadedWorld,
    regions: &[PackageRegion],
    opts: &LoadOptions,
) -> Vec<Candidate> {
    let touches = |m: &SignedManifest| {
        m.manifest.bounds.is_some_and(|b| {
            regions
                .iter()
                .any(|r| b.region().boxes_touch(r, TOUCH_TOLERANCE_DEG))
        })
    };
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: BTreeSet<String> = [home.identity.id.clone()].into();
    // The cache: the newest version of each id.
    let mut entries = opts.cache.list();
    entries.sort_by(|a, b| a.id.cmp(&b.id).then(b.version.cmp(&a.version)));
    for e in entries {
        if seen.contains(&e.id) {
            continue;
        }
        seen.insert(e.id.clone());
        if e.manifest().is_ok_and(|m| touches(&m)) {
            out.push(Candidate {
                spec: format!("{}@{}", e.id, e.version),
                id: e.id,
            });
        }
    }
    // Packages next to a package loaded by path.
    let dir = home.path.parent().unwrap_or(Path::new("."));
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    for f in files {
        if f == home.path || f.extension().is_none_or(|x| x != "ocwpkg") {
            continue;
        }
        let Some(m) = manifest_path_for(&f)
            .and_then(|mp| std::fs::read_to_string(mp).ok())
            .and_then(|t| SignedManifest::parse(&t).ok())
        else {
            continue;
        };
        if seen.contains(&m.manifest.id) || !touches(&m) {
            continue;
        }
        seen.insert(m.manifest.id.clone());
        out.push(Candidate {
            spec: f.display().to_string(),
            id: m.manifest.id.clone(),
        });
    }
    out
}

/// The home package and its neighbours, ready to ride.
#[derive(Debug, Clone)]
pub struct Neighbourhood {
    pub set: WorldSet,
    /// Per member of the set: how to load it again (home first).
    pub specs: Vec<String>,
    /// Per member: what was loaded (a reload must be the same package).
    pub identities: Vec<PackageIdentity>,
    /// One line per neighbour joined, refused or not loadable.
    pub log: Vec<String>,
    /// The neighbours as loaded (manifest, trust) for credits and info.
    pub worlds: Vec<LoadedWorld>,
}

impl Neighbourhood {
    /// Loads member `i` again (verified; refused if it changed since).
    pub fn reload(&self, i: usize, opts: &LoadOptions) -> Result<WorldPackage, String> {
        let spec = self.specs.get(i).ok_or("no such package")?;
        let w = load_world(spec, opts).map_err(|e| e.to_string())?;
        if w.identity != self.identities[i] {
            return Err(format!(
                "{spec}: changed during the ride ({} instead of {})",
                w.identity, self.identities[i]
            ));
        }
        Ok(w.package)
    }
}

/// The world set of `home` with every installed neighbour that a road
/// crosses to (none when `enabled` is false).
pub fn neighbourhood(
    home: &LoadedWorld,
    home_spec: &str,
    opts: &LoadOptions,
    enabled: bool,
) -> Neighbourhood {
    let mut n = Neighbourhood {
        set: WorldSet::new(&home.identity.id, Arc::new(home.package.clone())),
        specs: vec![home_spec.to_string()],
        identities: vec![home.identity.clone()],
        log: Vec::new(),
        worlds: Vec::new(),
    };
    if !enabled {
        return n;
    }
    let Some(home_box) = home_region(home) else {
        return n;
    };
    // Breadth first: the packages touching the members joined so far.
    let mut regions = vec![home_box];
    let mut tried: BTreeSet<String> = BTreeSet::new();
    loop {
        let fresh: Vec<Candidate> = candidates_near(home, &regions, opts)
            .into_iter()
            .filter(|c| !tried.contains(&c.id))
            .collect();
        if fresh.is_empty() || n.set.len() >= MAX_MEMBERS {
            break;
        }
        let before = n.set.len();
        for c in fresh {
            if n.set.len() >= MAX_MEMBERS {
                n.log.push(format!(
                    "neighbour {}: not joined: more than {MAX_MEMBERS} packages",
                    c.spec
                ));
                break;
            }
            tried.insert(c.id.clone());
            if let Some(r) = join(&mut n, c, opts) {
                regions.push(r);
            }
        }
        if n.set.len() == before {
            break;
        }
    }
    n
}

/// Loads a candidate's skeleton and joins it if a road crosses; returns
/// its region box.
fn join(n: &mut Neighbourhood, c: Candidate, opts: &LoadOptions) -> Option<PackageRegion> {
    let mut w = match load_world_skeleton(&c.spec, opts) {
        Ok(w) => w,
        Err(e) => {
            n.log.push(format!("neighbour {}: not loaded: {e}", c.spec));
            return None;
        }
    };
    let region = w.package.region.clone();
    {
        match n
            .set
            .add_neighbour(&w.identity.id, Arc::new(w.package.clone()))
        {
            Ok(links) => {
                // Its tiles come when the rider comes near (renderer).
                let i = n.set.len() - 1;
                n.set.set_resident(i, None);
                w.package.graph = None;
                let at = n.set.members().last().map_or(String::new(), |m| {
                    let pts: Vec<String> = m
                        .crossings
                        .iter()
                        .take(24)
                        .map(|p| format!("{:.0},{:.0}", p[0], p[1]))
                        .collect();
                    format!(" at x,z {}", pts.join(" "))
                });
                n.log.push(format!(
                    "neighbour {} joined: {links} road(s) across the border{at}",
                    w.describe()
                ));
                n.specs.push(c.spec);
                n.identities.push(w.identity.clone());
                n.worlds.push(w);
                region
            }
            Err(e) => {
                n.log.push(format!("neighbour {}: not joined: {e}", c.spec));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldCache;
    use crate::cache::MANIFEST_FILE;
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
    use ocw_world::manifest::{Attribution, Manifest, TrustStore};
    use ocw_world::package::{Tile, TileKey};

    const NOW: i64 = 1_800_000_000;
    const ORIGIN: (f64, f64) = (47.0, 9.5);

    fn edge(id: u64, from: u32, to: u32, pts: [[f64; 2]; 2], boundary: bool) -> Edge {
        Edge {
            id: EdgeId(id),
            from: VertexId(from),
            to: VertexId(to),
            points: pts.to_vec(),
            class: RoadClass::Residential,
            name: None,
            width_m: 5.0,
            bridge: false,
            tunnel: false,
            dead_end_m: None,
            boundary,
            oneway: Oneway::No,
            osm_way: 7,
            profile: vec![],
        }
    }

    /// Half of a road along z = 0 cut at x = 0 (lon 9.5): the west half
    /// from x = −800, or the east half to x = +800. Shared origin.
    fn half(name: &str, west: bool) -> WorldPackage {
        let far = if west { -800.0 } else { 800.0 };
        let mut g = RoadGraph {
            vertices: vec![[0.0, 0.0], [far, 0.0]],
            edges: vec![edge(
                if west { 1 } else { 2 },
                0,
                1,
                [[0.0, 0.0], [far, 0.0]],
                true,
            )],
            boundary_vertices: Default::default(),
        };
        g.boundary_vertices.insert(VertexId(0));
        let key = TileKey {
            x: if west { -1 } else { 0 },
            z: -1,
        };
        WorldPackage {
            name: name.into(),
            origin: ORIGIN,
            tile_size_m: 1000.0,
            samples: 2,
            tiles: [(
                key,
                Tile {
                    key,
                    heights: vec![0.0; 4],
                    roads: vec![],
                },
            )]
            .into(),
            graph: Some(g),
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![],
            region: Some(if west {
                PackageRegion::bbox(46.99, 9.48, 47.01, 9.5)
            } else {
                PackageRegion::bbox(46.99, 9.5, 47.01, 9.52)
            }),
            time_zone: None,
        }
    }

    /// Writes `pkg` with an unsigned manifest as `dir/file`.
    fn write(dir: &Path, file: &str, id: &str, pkg: &WorldPackage) -> std::path::PathBuf {
        let bytes = pkg.to_bytes();
        let mut m = Manifest::new(
            id,
            &pkg.name,
            1,
            NOW,
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        m.add_file(file, &bytes).unwrap();
        assert!(m.bounds.is_some());
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(file);
        std::fs::write(&path, &bytes).unwrap();
        let mut side = path.as_os_str().to_owned();
        side.push(".manifest.json");
        std::fs::write(side, SignedManifest::unsigned(&m).to_json()).unwrap();
        path
    }

    fn opts(cache: &Path) -> LoadOptions {
        LoadOptions {
            now_unix_s: NOW,
            // Unsigned test packages from the cache too.
            allow_unsigned: true,
            ..LoadOptions::new(TrustStore::default(), WorldCache::new(cache))
        }
    }

    #[test]
    fn a_package_next_to_the_home_package_is_joined() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("build");
        let home_path = write(&dir, "west.ocwpkg", "test/west", &half("West", true));
        write(&dir, "east.ocwpkg", "test/east", &half("East", false));
        // A copy of the home package under another file name (same id),
        // and one far away: not candidates.
        write(&dir, "west-old.ocwpkg", "test/west", &half("West", true));
        let mut far = half("Far", false);
        far.region = Some(PackageRegion::bbox(48.0, 9.5, 48.1, 9.6));
        write(&dir, "far.ocwpkg", "test/far", &far);
        let opts = opts(&tmp.path().join("cache"));
        let home = load_world(home_path.to_str().unwrap(), &opts).unwrap();
        let c = candidates(&home, &opts);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].id, "test/east");
        let n = neighbourhood(&home, home_path.to_str().unwrap(), &opts, true);
        assert_eq!(n.set.len(), 2, "{:?}", n.log);
        assert_eq!(n.set.name(), "West + East");
        assert_eq!(n.specs.len(), 2);
        assert_eq!(n.set.graph().unwrap().edges.len(), 2);
        // The border is linked; neither far end was a border.
        assert!(n.set.graph().unwrap().boundary_vertices.is_empty());
        // Reloading verifies again and must give the same package.
        assert_eq!(n.reload(1, &opts).unwrap().name, "East", "{:?}", n.specs);
        std::fs::write(
            dir.join("east.ocwpkg"),
            half("East changed", false).to_bytes(),
        )
        .unwrap();
        assert!(n.reload(1, &opts).is_err(), "hash mismatch / changed");
        // Disabled: the home package alone.
        let alone = neighbourhood(&home, home_path.to_str().unwrap(), &opts, false);
        assert_eq!(alone.set.len(), 1);
    }

    /// Package `k` of a row along lat 47: lon 9.48 + 0.02·k … + 0.02, with
    /// the road along z = 0 from border to border (cut at both).
    fn strip(k: u32) -> WorldPackage {
        let proj = ocw_world::geo::LocalProjection::new(ORIGIN.0, ORIGIN.1);
        let (w, e) = (9.48 + 0.02 * f64::from(k), 9.50 + 0.02 * f64::from(k));
        let (a, b) = (proj.project(47.0, w), proj.project(47.0, e));
        let mut g = RoadGraph {
            vertices: vec![a, b],
            edges: vec![edge(100 + u64::from(k), 0, 1, [a, b], true)],
            boundary_vertices: [VertexId(0), VertexId(1)].into(),
        };
        g.edges[0].points = vec![a, [(a[0] + b[0]) / 2.0, 0.0], b];
        let mut tiles = std::collections::BTreeMap::new();
        for x in (a[0] / 1000.0).floor() as i32..=(b[0] / 1000.0).floor() as i32 {
            let key = TileKey { x, z: -1 };
            tiles.insert(
                key,
                Tile {
                    key,
                    heights: vec![1.0; 4],
                    roads: vec![],
                },
            );
        }
        WorldPackage {
            name: format!("P{k}"),
            origin: ORIGIN,
            tile_size_m: 1000.0,
            samples: 2,
            tiles,
            graph: Some(g),
            landuse: None,
            buildings: None,
            bridges: None,
            time_zone: None,
            routes: vec![],
            region: Some(PackageRegion::bbox(46.99, w, 47.01, e)),
            time_zone: None,
        }
    }

    #[test]
    fn a_row_of_packages_rides_as_one_world_with_skeleton_neighbours() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("row");
        let home_path = write(&dir, "p0.ocwpkg", "test/p0", &strip(0));
        write(&dir, "p1.ocwpkg", "test/p1", &strip(1));
        write(&dir, "p2.ocwpkg", "test/p2", &strip(2));
        let opts = opts(&tmp.path().join("cache"));
        let home = load_world(home_path.to_str().unwrap(), &opts).unwrap();
        // Only the middle package touches the home package …
        assert_eq!(candidates(&home, &opts).len(), 1);
        // … but the far one is joined through it.
        let n = neighbourhood(&home, home_path.to_str().unwrap(), &opts, true);
        assert_eq!(n.set.name(), "P0 + P1 + P2", "{:?}", n.log);
        let g = n.set.graph().unwrap();
        assert_eq!(g.edges.len(), 3);
        // The two inner borders are linked, the two outer ends stay borders.
        assert_eq!(g.boundary_vertices.len(), 2, "{:?}", n.log);
        // Neighbours are joined without their tiles; the home package keeps them.
        assert!(n.set.members()[0].is_resident());
        assert!(!n.set.members()[1].is_resident() && !n.set.members()[2].is_resident());
        assert!(n.worlds.iter().all(|w| !w.package.has_bulk()));
        // Their tiles load in full when the rider comes near.
        let full = n.reload(2, &opts).unwrap();
        assert!(full.has_bulk());
        let mut set = n.set.clone();
        set.set_resident(2, Some(Arc::new(full)));
        assert!(set.members()[2].is_resident());
    }

    #[test]
    fn skeletons_keep_graph_routes_region_and_tile_keys() {
        let pkg = strip(1);
        let s = WorldPackage::skeleton_from_bytes(&pkg.to_bytes()).unwrap();
        assert_eq!(s.graph, pkg.graph);
        assert_eq!(s.region, pkg.region);
        assert_eq!(
            s.tiles.keys().collect::<Vec<_>>(),
            pkg.tiles.keys().collect::<Vec<_>>()
        );
        assert!(!s.has_bulk() && pkg.has_bulk());
    }

    #[test]
    fn neighbours_in_the_cache_are_found_by_their_box() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let opts = opts(&cache);
        // Install the east half into the cache (as a download would).
        let staged = tmp.path().join("staged");
        write(&staged, "east.ocwpkg", "test/east", &half("East", false));
        let side = staged.join("east.ocwpkg.manifest.json");
        std::fs::rename(&side, staged.join(MANIFEST_FILE)).unwrap();
        opts.cache.commit(&staged, "test/east", 1).unwrap();
        let home_path = write(
            &tmp.path().join("b"),
            "w.ocwpkg",
            "test/west",
            &half("West", true),
        );
        let home = load_world(home_path.to_str().unwrap(), &opts).unwrap();
        let c = candidates(&home, &opts);
        assert_eq!(
            c,
            vec![Candidate {
                spec: "test/east@1".into(),
                id: "test/east".into()
            }]
        );
        let n = neighbourhood(&home, home_path.to_str().unwrap(), &opts, true);
        assert_eq!(n.set.len(), 2, "{:?}", n.log);
        // The cache policy applies: without --allow-unsigned it is refused.
        let strict = LoadOptions {
            allow_unsigned: false,
            ..opts.clone()
        };
        let n = neighbourhood(&home, home_path.to_str().unwrap(), &strict, true);
        assert_eq!(n.set.len(), 1);
        assert!(n.log[0].contains("not loaded"), "{:?}", n.log);
    }
}
