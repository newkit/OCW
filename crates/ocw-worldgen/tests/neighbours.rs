//! Neighbouring packages end to end (spec §2.2.2, §4.4 step 4, §4.7):
//! the OSM fixture of `e2e.rs` is split at a meridian into a west and an
//! east package, as `world-builder package --bbox … --origin …` does. Both
//! cut the crossing roads at the same points; the client finds the east
//! package next to the west one by its manifest box, joins the road
//! graphs, and a bot on the headless server rides from the west package
//! into the east one instead of turning at the border.

use std::path::Path;
use std::sync::Arc;

use ocw_server::{Course, GameServer, ServerConfig};
use ocw_world::manifest::{Attribution, Manifest, SignedManifest, TrustStore};
use ocw_world::path::{RidePath, TurnReason};
use ocw_world::{RoadNetwork, Track};
use ocw_worldgen::build::{Region, build_road_graph};
use ocw_worldgen::dem::{Dem, HgtTile};
use ocw_worldgen::osm::OsmData;
use ocw_worldgen::profile::add_profiles;
use ocw_worldstore::{LoadOptions, WorldCache, load_world, neighbourhood};

const NOW: i64 = 1_800_000_000;
/// The border: between nodes 3 (9.5000) and 4 (9.5050) of Hauptstrasse.
const BORDER_LON: f64 = 9.5025;

/// A gentle synthetic slope (DEM tile N46E009).
fn dem() -> Dem {
    let n = 3601usize;
    let mut bytes = Vec::with_capacity(n * n * 2);
    for row in 0..n {
        let lat = 47.0 - row as f64 / (n - 1) as f64;
        for col in 0..n {
            let lon = 9.0 + col as f64 / (n - 1) as f64;
            let h = 500.0 + 2000.0 * (lat - 46.49) + 1500.0 * (lon - 9.49);
            bytes.extend((h.round() as i16).to_be_bytes());
        }
    }
    Dem::from_hgt(vec![HgtTile::from_bytes(46, 9, &bytes).unwrap()])
}

/// Builds one half as `world-builder package` does; writes the package
/// and an unsigned manifest (with the region box) into `dir`.
fn build_half(
    dir: &Path,
    file: &str,
    id: &str,
    region: Region,
    dem: &Dem,
) -> ocw_world::WorldPackage {
    let osm = OsmData::parse_osm_xml(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini-v1.osm"),
        )
        .unwrap(),
    )
    .unwrap();
    let proj = region.projection();
    let (mut graph, stats) = build_road_graph(&osm, &region);
    assert!(stats.boundary_edges >= 3, "{file}: {stats:?}");
    add_profiles(&mut graph, dem, &proj);
    // As `world-builder package --bbox`: the roads around the part (here
    // the whole fixture) give the profiles and the terrain.
    let context = Region {
        origin: Some(region.origin()),
        ..Region::bbox(46.490, 9.485, 46.510, 9.515)
    };
    let (mut around, _) = build_road_graph(&osm, &context);
    add_profiles(&mut around, dem, &proj);
    assert_eq!(
        ocw_worldgen::profile::transfer_profiles(&mut graph, &around, 0.5),
        graph.edges.len()
    );
    let base = |x: f64, z: f64| {
        let (lat, lon) = proj.unproject([x, z]);
        dem.elevation(lat, lon).unwrap_or(0.0)
    };
    let mut pkg = ocw_worldgen::package::build_package_with_terrain(
        file,
        region.origin(),
        &graph,
        &around,
        base,
        1000.0,
        51,
        1,
    );
    pkg.region = Some(region.package_region());
    let bytes = pkg.to_bytes();
    let mut m =
        Manifest::new(id, file, 1, NOW, "test", vec![Attribution::openstreetmap()]).unwrap();
    m.add_file(file, &bytes).unwrap();
    assert!(m.bounds.is_some());
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), &bytes).unwrap();
    std::fs::write(
        dir.join(format!("{file}.manifest.json")),
        SignedManifest::unsigned(&m).to_json(),
    )
    .unwrap();
    pkg
}

fn west() -> Region {
    Region::bbox(46.490, 9.485, 46.510, BORDER_LON)
}

fn east() -> Region {
    Region::bbox(46.490, BORDER_LON, 46.510, 9.515)
}

fn ride_across(origin_east: Option<(f64, f64)>) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("worlds");
    let dem = dem();
    let shared = Some((46.5, 9.5));
    let w = build_half(
        &dir,
        "west.ocwpkg",
        "test/west",
        Region {
            origin: shared,
            ..west()
        },
        &dem,
    );
    let e = build_half(
        &dir,
        "east.ocwpkg",
        "test/east",
        Region {
            origin: origin_east,
            ..east()
        },
        &dem,
    );
    let opts = LoadOptions {
        now_unix_s: NOW,
        ..LoadOptions::new(TrustStore::default(), WorldCache::new(tmp.path().join("c")))
    };
    let path = dir.join("west.ocwpkg");
    let spec = path.to_str().unwrap();
    let home = load_world(spec, &opts).unwrap();
    let hood = neighbourhood(&home, spec, &opts, true);
    assert_eq!(hood.set.len(), 2, "{:?}", hood.log);
    // Hauptstrasse, Einbahnweg and Brückenstrasse cross the border.
    assert_eq!(hood.set.members()[1].links, 3, "{:?}", hood.log);
    let joined = hood.set.graph().unwrap().clone();
    let alone = w.graph.clone().unwrap();
    assert_eq!(
        joined.boundary_vertices.len(),
        alone.boundary_vertices.len() - 3,
        "the three crossings are no border any more"
    );
    let net = Arc::new(RoadNetwork::new(joined).with_package_name(hood.set.name()));
    let proj = Region {
        origin: shared,
        ..west()
    }
    .projection();
    let border_x = proj.project(46.5, BORDER_LON)[0];
    // The two meet without a step: along the border line, the terrain of
    // both packages and the road heights where the roads cross agree.
    let f = hood.set.members()[1].frame;
    for k in 0..=40 {
        let p = [border_x, -900.0 + 45.0 * f64::from(k)];
        let q = f.invert(p);
        if let (Some(a), Some(b)) = (w.surface_height(p[0], p[1]), e.surface_height(q[0], q[1])) {
            assert!((a - b).abs() < 0.3, "terrain at {p:?}: {a:.2} vs {b:.2}");
        }
    }
    for c in &hood.set.members()[1].crossings {
        let near = |g: &ocw_world::graph::RoadGraph| -> f64 {
            let e = g
                .edges
                .iter()
                .find(|e| {
                    e.boundary
                        && [e.points[0], *e.points.last().unwrap()]
                            .iter()
                            .any(|p| (p[0] - c[0]).hypot(p[1] - c[1]) < 1.0)
                })
                .unwrap();
            if (e.points[0][0] - c[0]).hypot(e.points[0][1] - c[1]) < 1.0 {
                e.profile[0]
            } else {
                *e.profile.last().unwrap()
            }
        };
        let (a, b) = (near(w.graph.as_ref().unwrap()), {
            let mut g = e.graph.clone().unwrap();
            for v in &mut g.vertices {
                *v = f.apply(*v);
            }
            for ed in &mut g.edges {
                for p in &mut ed.points {
                    *p = f.apply(*p);
                }
            }
            near(&g)
        });
        assert!((a - b).abs() < 0.05, "road at {c:?}: {a:.2} vs {b:.2}");
    }
    // Hauptstrasse from junction 3 east to the border.
    let west_net = RoadNetwork::new(alone);
    let start = |net: &RoadNetwork| {
        let (edge, offset) = net.nearest(proj.project(46.5, 9.5010)).unwrap();
        let e = net.edge(edge);
        let towards_east =
            net.graph().vertices[e.to.0 as usize][0] > net.graph().vertices[e.from.0 as usize][0];
        (edge, offset, towards_east)
    };
    // Alone, the rider turns at the border.
    let alone = Arc::new(west_net);
    let (edge, offset, fwd) = start(&alone);
    let mut p = RidePath::start(alone.clone(), edge, offset, fwd, 0.0, 0);
    p.extend(0.0, &mut |j| j.default);
    let (at, reason) = p.turnaround_ahead(0.0).unwrap();
    assert_eq!(reason, TurnReason::Boundary);
    assert!(at < 200.0, "{at}");
    // Joined, a bot rides on into the east package.
    let (edge, offset_m, forward) = start(&net);
    let mut server = GameServer::new(
        Course::FreeRide {
            net: net.clone(),
            start: ocw_sim::FreeRideStart {
                edge,
                offset_m,
                forward,
            },
        },
        ServerConfig {
            allow_dev_commands: false,
            bots: 1,
            start_m: 0.0,
            allow_local_events: false,
            voice: None,
            ..Default::default()
        },
    );
    let bot = server.session().snapshot()[0].id;
    let mut max_x = f64::MIN;
    for _ in 0..(60 * ocw_core::SIM_TICK_HZ) {
        server.tick();
        let s = server.session();
        let d = s.rider_state(bot).unwrap().distance;
        if let Some(path) = s.path(bot) {
            max_x = max_x.max(path.sample(d).position[0]);
        }
    }
    assert!(
        max_x > border_x + 100.0,
        "the bot crossed the border at x = {border_x:.0} (got to {max_x:.0})"
    );
}

#[test]
fn a_ride_crosses_into_the_neighbour_with_a_shared_origin() {
    ride_across(Some((46.5, 9.5)));
}

#[test]
fn a_ride_crosses_into_the_neighbour_with_its_own_origin() {
    ride_across(None);
}
