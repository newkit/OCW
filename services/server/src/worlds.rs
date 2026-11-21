//! The worlds a headless server can switch to (spec §3.2.1, World →
//! Change world in a shared session): its catalog is what
//! [`ocw_worldstore::discover`] finds – the cache and the packages next to
//! `--world` – and a [`crate::WorldSwitch`] names one of them by id,
//! `id@version`, name or file name. A client never makes the server load
//! a path of its choosing.

use std::sync::Arc;

use ocw_protocol::v1;
use ocw_world::RoadNetwork;
use ocw_worldstore::discover::WorldEntry;
use ocw_worldstore::{LoadOptions, load_world};

use crate::Course;

/// The catalog entry a request means: an exact id, `id@version` or spec
/// first, then the name or file name (ignoring case). Broken or blocked
/// packages are refused with their reason; parts of a world count too.
pub fn resolve<'a>(entries: &'a [WorldEntry], wanted: &str) -> Result<&'a WorldEntry, String> {
    let wanted = wanted.trim();
    let all: Vec<&WorldEntry> = entries.iter().flat_map(WorldEntry::members).collect();
    let stem = |e: &WorldEntry| {
        e.path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_lowercase())
    };
    let exact = all.iter().find(|e| {
        e.id == wanted || e.spec == wanted || format!("{}@{}", e.id, e.version) == wanted
    });
    let w = wanted.to_lowercase();
    let loose = || {
        all.iter().find(|e| {
            e.name.to_lowercase() == w
                || stem(e).as_deref() == Some(w.as_str())
                || stem(e).as_deref() == Some(w.trim_end_matches(".ocwpkg"))
        })
    };
    let Some(e) = exact.or_else(loose) else {
        let names: Vec<String> = all
            .iter()
            .filter(|e| e.is_rideable())
            .map(|e| format!("{} ({})", e.id, e.name))
            .collect();
        return Err(format!(
            "no world {wanted:?} (known: {})",
            if names.is_empty() {
                "none".into()
            } else {
                names.join(", ")
            }
        ));
    };
    match &e.blocked {
        Some(why) => Err(format!("{}: {why}", e.name)),
        None => Ok(e),
    }
}

/// Loads `spec` (verified, with its installed neighbours) and builds the
/// course: the route `route` of the package, else a free ride from a start
/// chosen by `seed`; with the [`v1::WorldChange`] that tells the clients.
pub fn load_course(
    spec: &str,
    route: Option<&str>,
    opts: &LoadOptions,
    neighbours: bool,
    seed: u64,
) -> Result<(Course, v1::WorldChange), String> {
    load_course_at(spec, route, opts, neighbours, seed).map(|(c, w, _)| (c, w))
}

/// [`load_course`] with the package's geographic origin (lat, lon) – where
/// the server fetches real weather (spec §14 Phase 9, S7).
pub fn load_course_at(
    spec: &str,
    route: Option<&str>,
    opts: &LoadOptions,
    neighbours: bool,
    seed: u64,
) -> Result<(Course, v1::WorldChange, (f64, f64)), String> {
    let world = load_world(spec, opts).map_err(|e| match e {
        // A rider's own world whose key this server was not given (spec
        // §4.7 *User keys*): the operator decides, never a client.
        ocw_worldstore::LoadError::Verify(_, ocw_world::manifest::VerifyError::UntrustedKey(id)) => {
            format!(
                "this server does not trust the user key that signed it (key {id}); its operator can trust it with `ocw-server --trust-key` (spec §4.7)"
            )
        }
        e => e.to_string(),
    })?;
    tracing::info!(world = %world.describe(), "world package");
    let hood = ocw_worldstore::neighbourhood(&world, spec, opts, neighbours);
    for line in &hood.log {
        tracing::info!("{line}");
    }
    let graph = hood
        .set
        .graph()
        .cloned()
        .ok_or("no road graph (rebuild it with world-builder package)")?;
    // The home package's obstacles (spec §14 Phase 12 K3; neighbours'
    // are not loaded on the server).
    let obstacles =
        ocw_world::obstacles::WorldObstacles::from_package(&world.package).map(Arc::new);
    if let Some(o) = &obstacles {
        tracing::info!(
            tiles = o.tiles().count(),
            mb = o.approx_bytes() as f64 / 1e6,
            "world obstacles (spec §14 Phase 12 K3)"
        );
    }
    let net = Arc::new(
        RoadNetwork::new(graph)
            .with_package_name(hood.set.name())
            .with_obstacles(obstacles),
    );
    let change = v1::WorldChange {
        package_id: world.identity.id.clone(),
        version: world.identity.version,
        name: world
            .manifest
            .as_ref()
            .map_or_else(|| world.package.name.clone(), |m| m.name.clone()),
        world: Some(ocw_protocol::world_id(&net)),
        route: route.unwrap_or_default().to_owned(),
        content_sha256: world.identity.content_hash.0.to_vec(),
        by: String::new(),
    };
    let pkg = &world.package;
    let origin = pkg.origin;
    if let Some(name) = route {
        let stored = pkg.route(name).ok_or_else(|| {
            let names: Vec<&str> = pkg.routes.iter().map(|r| r.name.as_str()).collect();
            format!("no route {name:?} (the package has: {})", names.join(", "))
        })?;
        let plan = stored
            .resolve(&net)
            .map_err(|e| format!("route {name:?} does not fit the road graph: {e}"))?;
        return Ok((
            Course::RouteRide {
                net,
                plan: Arc::new(plan),
            },
            change,
            origin,
        ));
    }
    let (edge, offset_m, forward) = net.random_start(seed).ok_or("empty road graph")?;
    Ok((
        Course::FreeRide {
            net,
            start: ocw_sim::FreeRideStart {
                edge,
                offset_m,
                forward,
            },
        },
        change,
        origin,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_worldstore::discover::{Source, Status};

    fn entry(id: &str, name: &str, file: &str, blocked: Option<&str>) -> WorldEntry {
        WorldEntry {
            spec: format!("/srv/{file}"),
            id: id.into(),
            version: 3,
            name: name.into(),
            path: Some(format!("/srv/{file}").into()),
            source: Source::Folder,
            size_bytes: 1,
            created_unix_s: None,
            status: blocked.map_or(Status::Unsigned, |b| Status::Broken(b.into())),
            blocked: blocked.map(Into::into),
            summary: None,
            attributions: Vec::new(),
            parts: Vec::new(),
            also: Vec::new(),
        }
    }

    /// A package with a road graph (one straight edge, one tile).
    fn graph_package(name: &str) -> Vec<u8> {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
        use ocw_world::package::{Tile, TileKey};
        let key = TileKey { x: 0, z: 0 };
        ocw_world::WorldPackage {
            name: name.into(),
            origin: (47.14, 9.52),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: [(
                key,
                Tile {
                    key,
                    heights: vec![450.0; 4],
                    roads: Vec::new(),
                },
            )]
            .into(),
            graph: Some(
                RoadGraph {
                    vertices: vec![[100.0, 500.0], [900.0, 500.0]],
                    edges: vec![Edge {
                        id: EdgeId(7),
                        from: VertexId(0),
                        to: VertexId(1),
                        points: vec![[100.0, 500.0], [900.0, 500.0]],
                        class: RoadClass::Secondary,
                        name: None,
                        width_m: 6.0,
                        bridge: false,
                        tunnel: false,
                        dead_end_m: None,
                        boundary: false,
                        oneway: Oneway::No,
                        osm_way: 1,
                        profile: Vec::new(),
                    }],
                    boundary_vertices: Default::default(),
                }
                .into(),
            ),
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
        }
        .to_bytes()
    }

    /// The host asks for a rider's own world (spec §4.7 *User keys*): the
    /// server loads it only when its operator trusts the key – a client
    /// never makes it trust one.
    #[test]
    fn a_user_signed_world_needs_the_servers_own_trust() {
        use ocw_world::manifest::{
            Attribution, Manifest, RootKeyPair, SignedManifest, SigningKeyPair, TrustStore,
        };
        let now = 1_800_000_000;
        let root = RootKeyPair::from_seed("Ada Rider", [121; 32]);
        let cert = root.certify(
            "Ada Rider",
            SigningKeyPair::public_key_of([122; 32]),
            now - 10,
            now + 1000,
        );
        let key = SigningKeyPair::new([122; 32], cert)
            .unwrap()
            .with_root(root.public())
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let bytes = graph_package("Ada's Strip");
        let mut m = Manifest::new(
            "user/ada/strip",
            "Ada's Strip",
            1,
            now,
            "test",
            vec![Attribution::openstreetmap()],
        )
        .unwrap();
        m.add_file("strip.ocwpkg", &bytes).unwrap();
        let p = dir.path().join("strip.ocwpkg");
        std::fs::write(&p, &bytes).unwrap();
        std::fs::write(
            dir.path().join("strip.ocwpkg.manifest.json"),
            SignedManifest::signed(&m, &key).to_json(),
        )
        .unwrap();
        let opts = LoadOptions {
            now_unix_s: now,
            // Not even with --allow-unsigned.
            allow_unsigned: true,
            ..LoadOptions::new(
                TrustStore::default(),
                ocw_worldstore::WorldCache::new(dir.path().join("cache")),
            )
        };
        let spec = p.to_str().unwrap();
        let e = load_course(spec, None, &opts, false, 1).err().unwrap();
        assert!(
            e.contains("does not trust") && e.contains("--trust-key"),
            "{e}"
        );
        let trusting = LoadOptions {
            trust: ocw_worldstore::userkeys::with_user_keys(
                TrustStore::default(),
                &[root.public()],
            ),
            ..opts
        };
        let (_, change) = load_course(spec, None, &trusting, false, 1).unwrap();
        assert_eq!(
            (change.package_id.as_str(), change.version),
            ("user/ada/strip", 1)
        );
    }

    #[test]
    fn requests_name_a_world_of_the_catalog() {
        let mut li = entry(
            "europe/liechtenstein",
            "Liechtenstein",
            "liechtenstein.ocwpkg",
            None,
        );
        li.parts.push(entry(
            "europe/graubuenden",
            "Graubuenden",
            "gr.ocwpkg",
            None,
        ));
        let entries = vec![
            li,
            entry(
                "europe/blackforest",
                "Black Forest",
                "blackforest.ocwpkg",
                None,
            ),
            entry(
                "local/zip",
                "zip",
                "zip.ocwpkg",
                Some("not an OCW world package"),
            ),
        ];
        let id = |w: &str| resolve(&entries, w).map(|e| e.id.clone());
        assert_eq!(id("europe/blackforest").unwrap(), "europe/blackforest");
        assert_eq!(id("europe/blackforest@3").unwrap(), "europe/blackforest");
        assert_eq!(id("black forest").unwrap(), "europe/blackforest");
        assert_eq!(id("blackforest.ocwpkg").unwrap(), "europe/blackforest");
        assert_eq!(id(" Liechtenstein ").unwrap(), "europe/liechtenstein");
        assert_eq!(id("graubuenden").unwrap(), "europe/graubuenden", "a part");
        let err = id("zip").unwrap_err();
        assert!(err.contains("not an OCW world package"), "{err}");
        // Paths the client names are not loaded unless listed.
        let err = id("/etc/passwd").unwrap_err();
        assert!(
            err.contains("no world \"/etc/passwd\"") && err.contains("europe/blackforest"),
            "{err}"
        );
    }
}
