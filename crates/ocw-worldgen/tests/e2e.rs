//! End-to-end world pipeline (spec §14 Phase 3 integration tests): build a
//! small region from a checked-in OSM fixture and a synthetic DEM, package
//! and sign it, verify it, install it into a cache and load it as the
//! client does, ride it on the headless server with a bot through the
//! junction; then rebuild from changed OSM data and check which stable
//! edge IDs survive (the precondition for re-matching routes).
//!
//! No network, GPU or pilot data needed; runs in a few seconds.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use ocw_server::{Course, GameServer, ServerConfig};
use ocw_world::RoadNetwork;
use ocw_world::graph::{EdgeId, Oneway, RoadGraph};
use ocw_world::manifest::{
    Attribution, Manifest, RootKeyPair, SignedManifest, SigningKeyPair, TrustStore, UnsignedPolicy,
    VerifyError,
};
use ocw_worldgen::build::{Region, build_road_graph};
use ocw_worldgen::dem::{Dem, HgtTile};
use ocw_worldgen::diff::GraphDiff;
use ocw_worldgen::osm::OsmData;
use ocw_worldgen::profile::add_profiles;
use ocw_worldstore::cache::MANIFEST_FILE;
use ocw_worldstore::keys::{read_signing_key, write_new};
use ocw_worldstore::{FileTransport, LoadError, LoadOptions, WorldCache, fetch, load_world};

const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
/// The bridge (way 103) spans a valley along this meridian.
const VALLEY_LON: f64 = 9.5055;

/// Synthetic 1" DEM tile N46E009: a gentle slope (about 2 % up to the
/// north and east) and, south of the main road, a 25 m deep valley under
/// the bridge.
fn dem() -> Dem {
    let n = 3601usize;
    let mut bytes = Vec::with_capacity(n * n * 2);
    for row in 0..n {
        let lat = 47.0 - row as f64 / (n - 1) as f64;
        for col in 0..n {
            let lon = 9.0 + col as f64 / (n - 1) as f64;
            bytes.extend((terrain(lat, lon).round() as i16).to_be_bytes());
        }
    }
    Dem {
        tiles: vec![HgtTile::from_bytes(46, 9, &bytes).unwrap()],
    }
}

fn terrain(lat: f64, lon: f64) -> f64 {
    let base = 500.0 + 2000.0 * (lat - 46.49) + 1500.0 * (lon - 9.49);
    let fade = ((46.4985 - lat) / 0.001).clamp(0.0, 1.0);
    let valley = 25.0 * (-((lon - VALLEY_LON) / 0.0008).powi(2)).exp() * fade;
    base - valley
}

fn region() -> Region {
    Region::bbox(46.490, 9.485, 46.510, 9.515)
}

/// The world builder's pipeline: OSM → graph → profiles → package bytes.
fn build(osm_xml: &str, dem: &Dem) -> (RoadGraph, Vec<u8>) {
    let osm = OsmData::parse_osm_xml(osm_xml).unwrap();
    let region = region();
    let proj = region.projection();
    let (mut graph, stats) = build_road_graph(&osm, &region);
    assert_eq!(stats.components, 1, "{stats:?}");
    add_profiles(&mut graph, dem, &proj);
    let base = |x: f64, z: f64| {
        let (lat, lon) = proj.unproject([x, z]);
        dem.elevation(lat, lon).unwrap_or(0.0)
    };
    let pkg =
        ocw_worldgen::package::build_package("Mini", region.center(), &graph, base, 1000.0, 51, 1);
    (graph, pkg.to_bytes())
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn edges_of_way(g: &RoadGraph, way: i64) -> Vec<usize> {
    (0..g.edges.len())
        .filter(|&i| g.edges[i].osm_way == way)
        .collect()
}

/// Root and signing key files in `dir`, as `world-builder keys` writes them.
fn make_keys(dir: &Path) -> (RootKeyPair, std::path::PathBuf) {
    let root = RootKeyPair::from_seed("E2E test root", [11; 32]);
    let seed = [12; 32];
    let cert = root.certify(
        "e2e signing",
        SigningKeyPair::public_key_of(seed),
        NOW - DAY,
        NOW + 365 * DAY,
    );
    let signing = SigningKeyPair::new(seed, cert).unwrap();
    write_new(&dir.join("root.key"), &root.to_json(), true).unwrap();
    let key = dir.join("e2e.key");
    write_new(&key, &signing.to_json(), true).unwrap();
    (root, key)
}

#[test]
fn small_region_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let dem = dem();

    // 1. Build from the fixture; the graph has what the fixture promises.
    let (graph, bytes) = build(&fixture("mini-v1.osm"), &dem);
    assert_eq!(
        graph.edges.len(),
        7,
        "{:#?}",
        graph.edges.iter().map(|e| e.osm_way).collect::<Vec<_>>()
    );
    let proj = region().projection();
    let junction = graph.nearest_vertex(proj.project(46.5, 9.5)).unwrap();
    assert_eq!(
        graph.adjacency()[junction.0 as usize].len(),
        4,
        "four arms at node 3"
    );
    let sackgasse = &graph.edges[edges_of_way(&graph, 101)[0]];
    assert!(
        sackgasse.dead_end_m.is_some_and(|d| d > 300.0),
        "dead end flagged"
    );
    let bridge = &graph.edges[edges_of_way(&graph, 103)[0]];
    assert!(bridge.bridge);
    let oneway = &graph.edges[edges_of_way(&graph, 105)[0]];
    assert_ne!(oneway.oneway, Oneway::No);
    assert!(
        graph
            .edges
            .iter()
            .all(|e| e.dead_end_m.is_none() || e.osm_way == 101)
    );
    // The bridge runs straight over the valley instead of dipping into it.
    let mid = bridge.profile[bridge.profile.len() / 2];
    let ends = (bridge.profile[0] + bridge.profile.last().unwrap()) / 2.0;
    let valley_floor = terrain(46.495, VALLEY_LON);
    assert!(
        (mid - ends).abs() < 3.0,
        "bridge mid {mid:.1} vs ends {ends:.1}"
    );
    assert!(
        mid - valley_floor > 12.0,
        "bridge {mid:.1} over valley floor {valley_floor:.1}"
    );
    // Building is deterministic: same input, same bytes (same content hash
    // on every build machine, so multiplayer sessions can compare it).
    assert_eq!(build(&fixture("mini-v1.osm"), &dem).1, bytes);

    // 2. Package, manifest, signature (as `world-builder package --sign-key`).
    let keys_dir = tmp.path().join("keys");
    let (root, key_file) = make_keys(&keys_dir);
    let signing = read_signing_key(&key_file).unwrap();
    let mut manifest = Manifest::new(
        "test/mini",
        "Mini",
        1,
        NOW,
        "e2e test",
        vec![
            Attribution::openstreetmap(),
            Attribution {
                source: "Synthetic DEM".into(),
                licence: "CC0-1.0".into(),
                notice: "Elevation: synthetic test terrain".into(),
                url: None,
            },
        ],
    )
    .unwrap();
    manifest.add_file("mini.ocwpkg", &bytes).unwrap();
    assert!(
        manifest.files[0].chunks.len() >= 4,
        "header, tiles, sections"
    );
    let build_dir = tmp.path().join("build");
    std::fs::create_dir(&build_dir).unwrap();
    std::fs::write(build_dir.join("mini.ocwpkg"), &bytes).unwrap();
    std::fs::write(
        build_dir.join(MANIFEST_FILE),
        SignedManifest::signed(&manifest, &signing).to_json(),
    )
    .unwrap();

    // 3. Verify: trusted with our root, refused without it.
    let trust = TrustStore::new([root.public()]);
    let text = std::fs::read_to_string(build_dir.join(MANIFEST_FILE)).unwrap();
    let signed = SignedManifest::parse(&text).unwrap();
    signed.verify_signature(&trust, NOW).unwrap();
    signed.manifest.verify_file("mini.ocwpkg", &bytes).unwrap();
    assert!(matches!(
        signed.verify_signature(&TrustStore::default(), NOW),
        Err(VerifyError::UnknownRoot(_))
    ));

    // 4. Install into a cache (as `ocw-desktop --fetch-world DIR/manifest.json`)
    //    and load by id, verified, as the client and the server do.
    let cache = WorldCache::new(tmp.path().join("cache"));
    let fetched = fetch(
        &build_dir.join(MANIFEST_FILE).display().to_string(),
        &cache,
        &trust,
        UnsignedPolicy::Refuse,
        NOW,
        &FileTransport,
        &mut |_, _, _| {},
    )
    .unwrap();
    assert!(fetched.trust.is_trusted());
    let opts = LoadOptions {
        now_unix_s: NOW,
        ..LoadOptions::new(trust.clone(), cache.clone())
    };
    let world = load_world("test/mini", &opts).unwrap();
    assert!(world.trust.is_trusted(), "{}", world.trust);
    assert_eq!(world.identity, manifest.identity().unwrap());
    assert!(world.attributions()[0].contains("© OpenStreetMap contributors"));
    let loaded = world
        .package
        .graph
        .clone()
        .expect("version 2 package with graph");
    assert_eq!(loaded.edges.len(), graph.edges.len());
    assert!(world.package.tiles.len() >= 2);
    // The terrain follows the road (spec §4.3): no road under the terrain.
    let (_, _, above, total) = world.package.road_pokes(2.0);
    assert!(
        above * 1000 < total,
        "{above} of {total} road samples under terrain"
    );
    // A damaged cache entry is refused on load.
    let cached = fetched.entry.dir.join("mini.ocwpkg");
    let mut damaged = bytes.clone();
    *damaged.last_mut().unwrap() ^= 1;
    std::fs::write(&cached, &damaged).unwrap();
    assert!(matches!(
        load_world("test/mini", &opts),
        Err(LoadError::Verify(_, VerifyError::HashMismatch(_)))
    ));
    std::fs::write(&cached, &bytes).unwrap();

    // 5. Free ride on the headless server: a bot starts on Hauptstrasse
    //    west of the junction and rides through it.
    let net = Arc::new(RoadNetwork::new(loaded));
    let jx = net.graph().vertices[junction.0 as usize][0];
    let west = edges_of_way(net.graph(), 100)
        .into_iter()
        .find(|&i| {
            let e = net.edge(i);
            (e.from == junction || e.to == junction)
                && net.graph().vertices[e.other(junction).0 as usize][0] < jx
        })
        .expect("Hauptstrasse west of the junction");
    let e = net.edge(west);
    let forward = e.to == junction;
    let len = net.geometry(west).length();
    let offset_m = if forward { len - 250.0 } else { 250.0 };
    let mut server = GameServer::new(
        Course::FreeRide {
            net: net.clone(),
            start: ocw_sim::FreeRideStart {
                edge: west,
                offset_m,
                forward,
            },
        },
        ServerConfig {
            allow_dev_commands: false,
            bots: 1,
            start_m: 0.0,
            allow_local_events: false,
        },
    );
    let bot = server.session().snapshot()[0].id;
    let mut visited: Vec<EdgeId> = Vec::new();
    for _ in 0..(90 * ocw_core::SIM_TICK_HZ) {
        server.tick();
        let s = server.session();
        let d = s.rider_state(bot).unwrap().distance;
        if let Some((edge, _, _)) = s.path(bot).and_then(|p| p.locate(d)) {
            let id = net.edge(edge).id;
            if visited.last() != Some(&id) {
                visited.push(id);
            }
        }
    }
    let state = server.session().rider_state(bot).unwrap();
    assert!(state.distance > 400.0, "bot rode {:.0} m", state.distance);
    assert_eq!(visited[0], net.edge(west).id, "{visited:?}");
    let after = visited.get(1).expect("the bot passed the junction");
    let next = net.edge(net.edge_index(*after).unwrap());
    assert!(
        next.from == junction || next.to == junction,
        "the edge after the start leaves the junction"
    );
    eprintln!(
        "bot rode {:.0} m through the junction onto {} (edges {:?})",
        state.distance,
        next.name.as_deref().unwrap_or("unnamed"),
        visited
            .iter()
            .map(|id| net.edge(net.edge_index(*id).unwrap()).osm_way)
            .collect::<Vec<_>>()
    );

    // 6. Rebuild from changed OSM data: unchanged roads keep their stable
    //    edge IDs, so routes on them re-match; changed ones are reported.
    let (graph2, bytes2) = build(&fixture("mini-v2.osm"), &dem);
    assert_ne!(bytes2, bytes);
    let diff = GraphDiff::between(&graph, &graph2);
    eprintln!("rebuild: {}", diff.summary());
    let ways = |g: &RoadGraph, ids: &mut dyn Iterator<Item = EdgeId>| -> BTreeSet<i64> {
        ids.map(|id| g.edges.iter().find(|e| e.id == id).unwrap().osm_way)
            .collect()
    };
    assert_eq!(
        ways(&graph, &mut diff.unchanged.iter().copied()),
        [100, 101, 102, 103].into()
    );
    assert_eq!(
        diff.unchanged.len(),
        4,
        "Hauptstrasse west, Sackgasse, Brückenstrasse 5-9, bridge"
    );
    // Hauptstrasse east: node 4 moved 33 m, same node sequence, same ID.
    assert_eq!(diff.moved.len(), 1);
    assert_eq!(
        ways(&graph, &mut diff.moved.iter().map(|m| m.0)),
        [100].into()
    );
    assert!((20.0..50.0).contains(&diff.moved[0].1), "{:?}", diff.moved);
    // Brückenstrasse 10-1 re-routed (new node) and Einbahnweg deleted.
    assert_eq!(
        ways(&graph, &mut diff.removed.iter().copied()),
        [104, 105].into()
    );
    assert_eq!(ways(&graph2, &mut diff.added.iter().copied()), [104].into());
    // A route over the junction and the bridge re-matches; one over the
    // deleted one-way breaks there.
    let id = |g: &RoadGraph, way| g.edges[edges_of_way(g, way)[0]].id;
    let route = [id(&graph, 102), id(&graph, 103)];
    assert!(diff.broken(&route).is_empty());
    let route = [id(&graph, 100), id(&graph, 105), id(&graph, 103)];
    assert_eq!(diff.broken(&route), [&id(&graph, 105)]);

    // 7. The rebuilt package is a new version; the cache keeps both and
    //    the id resolves to the newest.
    let mut m2 = Manifest {
        version: 2,
        ..manifest.clone()
    };
    m2.files.clear();
    m2.add_file("mini.ocwpkg", &bytes2).unwrap();
    let dir2 = tmp.path().join("build2");
    std::fs::create_dir(&dir2).unwrap();
    std::fs::write(dir2.join("mini.ocwpkg"), &bytes2).unwrap();
    std::fs::write(
        dir2.join(MANIFEST_FILE),
        SignedManifest::signed(&m2, &signing).to_json(),
    )
    .unwrap();
    fetch(
        &dir2.join(MANIFEST_FILE).display().to_string(),
        &cache,
        &trust,
        UnsignedPolicy::Refuse,
        NOW,
        &FileTransport,
        &mut |_, _, _| {},
    )
    .unwrap();
    let newest = load_world("test/mini", &opts).unwrap();
    assert_eq!(newest.identity.version, 2);
    assert_ne!(newest.identity.content_hash, world.identity.content_hash);
    assert_eq!(
        load_world("test/mini@1", &opts).unwrap().identity,
        world.identity
    );
}
