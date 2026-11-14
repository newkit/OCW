//! Worlds over HTTP in the browser app (spec §14 Phase 10, B2): the page's
//! `?world=` is streamed with [`ocw_worldstore::stream`] – manifest and
//! signature chain first, then the base (road graph, routes, land use,
//! buildings …) and the tiles around the start with range requests, each
//! chunk verified against the manifest, cached in the browser's Origin
//! Private File System – while the loading screen shows the progress. The
//! ride starts when the first tiles are there; while riding, tiles within
//! the fetch radius come on demand and tiles beyond the keep radius (or
//! over the memory budget) are dropped ([`ResidentTiles`]).
//!
//! This module is the Bevy glue and runs natively in the tests, with the
//! driver stepped in each frame over an in-memory server; in the browser
//! the driver runs in its own `spawn_local` task over `fetch()` and OPFS
//! (`crate::browser`).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ocw_client::client::ClientCore;
use ocw_core::{RiderLook, RiderProfile};
use ocw_render::RideView;
use ocw_render::console::ConsoleState;
use ocw_render::package::PackageWorld;
use ocw_server::Course;
use ocw_world::manifest::{Hex, RootKey, TrustStore, UnsignedPolicy, key_id};
use ocw_world::package::TileKey;
use ocw_world::{RoadNetwork, WorldPackage, WorldSet};
use ocw_worldstore::stream::driver::{StreamBase, StreamConfig, StreamShared};
use ocw_worldstore::stream::plan::RangeParams;
use ocw_worldstore::stream::progress::{Stage, mb};
use ocw_worldstore::stream::resident::{Residency, ResidentTiles};
use ocw_worldstore::stream::{WorldRef, cache::DEFAULT_BUDGET};

use crate::plugin::{Place, Session};
use crate::session::{SessionConfig, WebSession};

/// Where the dev server (`make web-serve`) serves `world-builder publish`
/// output, relative to the page.
pub const WORLDS_BASE: &str = "worlds/";
/// Seconds between residency updates while riding.
pub const PLAN_INTERVAL_S: f64 = 0.25;
/// Seconds between two updates of the renderer's package (each rebuilds
/// the far terrain).
pub const PUBLISH_INTERVAL_S: f64 = 1.0;
/// Seconds between the streaming lines in the log (the browser test reads
/// them).
pub const LOG_INTERVAL_S: f64 = 2.0;

/// The page's world options (`?world=…&route=…`, see `crate::parse_query`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldOptions {
    /// `?world=`: a manifest URL, `ID@VERSION` or `ID`.
    pub spec: String,
    /// `?route=`: a route of the package (default: its first; none: free ride).
    pub route: Option<String>,
    /// `?free=1`: a free ride on the road graph.
    pub free_ride: bool,
    pub residency: Residency,
    /// The OPFS chunk cache's budget (bytes).
    pub cache_budget: u64,
    /// `?cache=0`: no OPFS cache.
    pub use_cache: bool,
    /// Dev pages only: accept unsigned packages (their chunks are still
    /// checked against the manifest's SHA-256s), trust the development
    /// roots, trust extra root public keys (hex).
    pub allow_unsigned: bool,
    pub trust_dev: bool,
    pub trust_roots: Vec<[u8; 32]>,
    /// Where worlds by id are found.
    pub base: String,
}

impl WorldOptions {
    pub fn new(spec: &str) -> Self {
        Self {
            spec: spec.into(),
            route: None,
            free_ride: false,
            residency: Residency::default(),
            cache_budget: DEFAULT_BUDGET,
            use_cache: true,
            allow_unsigned: false,
            trust_dev: false,
            trust_roots: Vec::new(),
            base: WORLDS_BASE.into(),
        }
    }

    /// The roots this page trusts: the production roots (none yet), the
    /// development roots and the page's extra roots (dev pages only).
    pub fn trust_store(&self) -> TrustStore {
        let mut trust = ocw_worldstore::trust::TrustConfig {
            dev_roots: self.trust_dev,
            root_files: Vec::new(),
            roots_dir: None,
        }
        .trust_store()
        .unwrap_or_default();
        for pk in &self.trust_roots {
            trust.roots.push(RootKey {
                kind: "ocw-root-public-key".into(),
                name: "a root of the page options (development)".into(),
                key_id: key_id(pk),
                public_key: Hex(*pk),
                dev_only: true,
            });
        }
        trust
    }

    /// The driver's configuration.
    pub fn stream_config(&self, now_unix_s: i64) -> Result<StreamConfig, String> {
        Ok(StreamConfig {
            world: self.spec.clone(),
            source: WorldRef::parse(&self.spec, &self.base)?,
            trust: self.trust_store(),
            policy: if self.allow_unsigned {
                UnsignedPolicy::Warn
            } else {
                UnsignedPolicy::Refuse
            },
            now_unix_s,
            ranges: RangeParams::default(),
            cache_budget: self.cache_budget,
            batch_tiles: 16,
        })
    }
}

/// A ride on the streamed package, ready to start.
pub struct WorldRide {
    pub course: Course,
    pub client: ClientCore,
    /// Where the rider starts (package metres, x / z).
    pub start: [f64; 2],
    /// One line for the log.
    pub description: String,
}

/// The ride the options ask for on `pkg` (its base: graph and routes): a
/// route of the package (`?route=`, default its first), else a free ride.
pub fn world_ride(
    pkg: &WorldPackage,
    opts: &WorldOptions,
    name: &str,
) -> Result<WorldRide, String> {
    let graph = pkg.graph.clone().ok_or_else(|| {
        format!(
            "{}: no road graph in this world package (format OCWPKG02 needed)",
            pkg.name
        )
    })?;
    let net = Arc::new(RoadNetwork::new(graph).with_package_name(pkg.name.clone()));
    let profile = RiderProfile::default();
    let route_name = match (&opts.route, opts.free_ride) {
        (_, true) => None,
        (Some(r), false) => Some(r.clone()),
        (None, false) => pkg.routes.first().map(|r| r.name.clone()),
    };
    if let Some(route_name) = route_name {
        let route = pkg
            .routes
            .iter()
            .find(|r| {
                r.name.eq_ignore_ascii_case(&route_name) || slug(&r.name) == slug(&route_name)
            })
            .ok_or_else(|| {
                let names: Vec<&str> = pkg.routes.iter().map(|r| r.name.as_str()).collect();
                format!(
                    "no route {route_name:?} in {} (it has: {})",
                    pkg.name,
                    if names.is_empty() {
                        "none".into()
                    } else {
                        names.join(", ")
                    }
                )
            })?;
        let plan = Arc::new(
            route
                .resolve(&net)
                .map_err(|e| format!("route {:?} does not fit the road graph: {e}", route.name))?,
        );
        let (edge, offset, _) = plan.start();
        let p = net.geometry(edge).point_at(offset);
        let description = format!(
            "route {} of {}: {:.1} km",
            route.name,
            pkg.name,
            plan.length_m() / 1000.0
        );
        let client = ClientCore::free_ride(net.clone(), name, profile)
            .with_look(RiderLook::anonymous())
            .with_geo_origin(pkg.origin)
            .with_route_plan(plan.clone());
        return Ok(WorldRide {
            course: Course::RouteRide { net, plan },
            client,
            start: [p[0], p[2]],
            description,
        });
    }
    let (edge, offset_m, forward) = net.random_start(1).ok_or("the road graph is empty")?;
    let p = net.geometry(edge).point_at(offset_m);
    let client = ClientCore::free_ride(net.clone(), name, profile)
        .with_look(RiderLook::anonymous())
        .with_geo_origin(pkg.origin);
    Ok(WorldRide {
        course: Course::FreeRide {
            net,
            start: ocw_sim::FreeRideStart {
                edge,
                offset_m,
                forward,
            },
        },
        client,
        start: [p[0], p[2]],
        description: format!("free ride in {}", pkg.name),
    })
}

/// `Vaduz–Malbun` → `vaduz-malbun` (route names in page links).
pub fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// The app side of a streamed world.
#[derive(Resource)]
pub struct WorldStream {
    pub shared: Arc<Mutex<StreamShared>>,
    pub options: WorldOptions,
    /// The rider's name and the session's bots and power, for the start.
    pub session: SessionConfig,
    pub resident: Option<ResidentTiles>,
    /// The package id and version (`liechtenstein@21`).
    pub identity: Option<String>,
    pending: Option<WorldRide>,
    pub started: bool,
    /// Tiles that failed (not asked for again).
    pub failed: BTreeSet<TileKey>,
    last_plan_s: f64,
    last_publish_s: f64,
    last_log_s: f64,
    /// The loading screen's lines (shown by the page while loading).
    pub screen: Vec<String>,
}

impl WorldStream {
    pub fn new(options: WorldOptions, session: SessionConfig) -> Self {
        let shared = StreamShared {
            progress: ocw_worldstore::stream::progress::LoadProgress::new(&options.spec),
            ..StreamShared::default()
        };
        Self {
            shared: Arc::new(Mutex::new(shared)),
            options,
            session,
            resident: None,
            identity: None,
            pending: None,
            started: false,
            failed: BTreeSet::new(),
            last_plan_s: f64::NEG_INFINITY,
            last_publish_s: f64::NEG_INFINITY,
            last_log_s: 0.0,
            screen: Vec::new(),
        }
    }

    /// The loading failed (the screen says why).
    pub fn failed_to_load(&self) -> Option<String> {
        let s = self.shared.lock().expect("not poisoned");
        (s.progress.stage == Stage::Failed && !self.started)
            .then(|| s.progress.error.clone().unwrap_or_default())
    }

    /// One line on the streaming state.
    pub fn status_line(&self, wasm_memory: Option<u64>) -> String {
        let Some(r) = &self.resident else {
            return "world streaming: loading".into();
        };
        let s = self.shared.lock().expect("not poisoned");
        let t = &s.tile_stats;
        format!(
            "world streaming: {} of {} tiles resident ({}, peak {}), {} loaded, {} dropped, {} pending; tiles from network {}, from cache {}{}",
            r.len(),
            r.tile_count(),
            mb(r.resident_bytes()),
            mb(r.peak_bytes()),
            r.loaded_total,
            r.dropped_total,
            r.pending(),
            mb(t.network_bytes),
            mb(t.cache_bytes),
            wasm_memory.map_or(String::new(), |m| format!(", wasm memory {}", mb(m)))
        )
    }

    fn request(&mut self, wanted: Vec<(TileKey, usize)>) -> usize {
        let wanted: Vec<(TileKey, usize)> = wanted
            .into_iter()
            .filter(|(k, _)| !self.failed.contains(k))
            .collect();
        let n = wanted.len();
        self.shared
            .lock()
            .expect("not poisoned")
            .requests
            .extend(wanted);
        n
    }
}

/// The base arrived: the ride is prepared and the first tiles requested.
fn on_base(stream: &mut WorldStream, base: StreamBase) -> Result<(), String> {
    let m = &base.manifest;
    stream.identity = Some(format!("{}@{}", m.id, m.version));
    info!(
        "world {}@{}: {} ({}), {} tiles, trust: {}",
        m.id,
        m.version,
        m.name,
        mb(base.file.size),
        base.package.head.tile_count,
        base.trust
    );
    let ride = world_ride(base.package.base(), &stream.options, &stream.session.name)?;
    info!("world ride: {}", ride.description);
    let mut resident = ResidentTiles::new(base.package, &base.file, stream.options.residency);
    let first = resident.update(ride.start);
    let n = stream.request(first);
    stream
        .shared
        .lock()
        .expect("not poisoned")
        .progress
        .tiles_wanted = n as u32;
    stream.resident = Some(resident);
    stream.pending = Some(ride);
    Ok(())
}

/// Moves the arrived tiles into the resident set.
fn take_arrived(stream: &mut WorldStream, console: Option<&mut ConsoleState>) {
    let arrived = std::mem::take(&mut stream.shared.lock().expect("not poisoned").arrived);
    let Some(resident) = stream.resident.as_mut() else {
        return;
    };
    let mut console = console;
    for (key, result) in arrived {
        let r = result.and_then(|bytes| resident.arrive(key, &bytes));
        if let Err(e) = r {
            resident.failed(key);
            stream.failed.insert(key);
            warn!("world tile {},{}: {e}", key.x, key.z);
            if let Some(c) = console.as_deref_mut() {
                c.print(format!("world tile {},{} not loaded: {e}", key.x, key.z));
            }
        }
    }
}

/// The streaming each frame: base → ride, arrived tiles, residency
/// around the rider, the renderer's package, the log line.
pub fn stream_world(world: &mut World) {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let Some(mut stream) = world.remove_resource::<WorldStream>() else {
        return;
    };
    let base = stream.shared.lock().expect("not poisoned").base.take();
    if let Some(base) = base
        && let Err(e) = on_base(&mut stream, base)
    {
        error!("world: {e}");
        let mut s = stream.shared.lock().expect("not poisoned");
        s.progress.stage = Stage::Failed;
        s.progress.error = Some(e);
    }
    {
        let mut console = world.get_resource_mut::<ConsoleState>();
        take_arrived(&mut stream, console.as_deref_mut());
    }
    if !stream.started {
        let ready = stream
            .resident
            .as_ref()
            .is_some_and(|r| r.pending() == 0 && !r.is_empty());
        if ready && let Some(ride) = stream.pending.take() {
            start_ride(world, &mut stream, ride, now);
        }
    } else if now - stream.last_plan_s >= PLAN_INTERVAL_S {
        stream.last_plan_s = now;
        let focus = world
            .get_resource::<RideView>()
            .and_then(ocw_render::flight_world::stream_focus);
        if let (Some(focus), Some(r)) = (focus, stream.resident.as_mut()) {
            let wanted = r.update(focus.centre);
            stream.request(wanted);
        }
        if now - stream.last_publish_s >= PUBLISH_INTERVAL_S
            && let Some(pkg) = stream
                .resident
                .as_mut()
                .and_then(ResidentTiles::take_package)
        {
            stream.last_publish_s = now;
            publish(world, pkg);
        }
    }
    {
        let s = stream.shared.lock().expect("not poisoned");
        stream.screen = s.progress.lines();
    }
    if stream.started && now - stream.last_log_s >= LOG_INTERVAL_S {
        stream.last_log_s = now;
        info!("{}", stream.status_line(crate::wasm_memory_bytes()));
    }
    world.insert_resource(stream);
}

/// New resident tiles for the renderer (no re-mesh of the tiles already
/// there; the far terrain is rebuilt from them).
fn publish(world: &mut World, pkg: WorldPackage) {
    let Some(mut pw) = world.get_resource_mut::<PackageWorld>() else {
        return;
    };
    Arc::make_mut(&mut pw.set).update_tiles(0, Arc::new(pkg));
    let ground = pw.ground();
    world.insert_resource(ground);
}

/// Starts the ride: the session on the package's road graph, the world
/// for the renderer, the scene rebuilt.
fn start_ride(world: &mut World, stream: &mut WorldStream, ride: WorldRide, now: f64) {
    let Some(pkg) = stream
        .resident
        .as_mut()
        .and_then(ResidentTiles::take_package)
    else {
        return;
    };
    stream.started = true;
    stream.last_publish_s = now;
    let id = stream.identity.clone().unwrap_or_default();
    let (zone, _) = ocw_client::sky::place_zone(pkg.time_zone.as_deref(), Some(pkg.origin));
    let pkg = Arc::new(pkg);
    world.insert_resource(PackageWorld {
        set: Arc::new(WorldSet::new(&id, pkg)),
        loader: None,
    });
    // The local server, or a remote one (B3) on the same package.
    let transport = crate::plugin::MakeTransport::for_config(world, &stream.session);
    let config = SessionConfig {
        start_m: 0.0,
        ..stream.session.clone()
    };
    let session = WebSession::start(ride.course, ride.client, &config, transport);
    world.insert_resource(Session(session));
    world.insert_resource(Place(zone));
    if let Some(mut view) = world.get_resource_mut::<RideView>() {
        view.free_ride = true;
        view.riders.clear();
    }
    ocw_render::rebuild_scene(world);
    let summary = {
        let mut s = stream.shared.lock().expect("not poisoned");
        if s.progress.stage == Stage::FirstTiles {
            s.progress.stage = Stage::Riding;
        }
        s.progress.summary()
    };
    info!("{summary}");
    info!("{}", stream.status_line(crate::wasm_memory_bytes()));
}

/// The world plugin: the stream resource and its system (the driver is
/// started by the platform: `crate::browser` in the browser, the tests'
/// [`NativeDriver`] natively).
pub struct WebWorldPlugin {
    pub options: WorldOptions,
    pub session: SessionConfig,
}

impl Plugin for WebWorldPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WorldStream::new(self.options.clone(), self.session.clone()))
            .add_systems(PreUpdate, stream_world);
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(PreUpdate, drive_native.before(stream_world));
    }
}

/// Natively (tests): a driver over an in-memory server, stepped until idle
/// each frame.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub struct NativeDriver(
    pub  Mutex<
        ocw_worldstore::stream::driver::StreamDriver<
            ocw_worldstore::stream::MemoryFetch,
            Arc<ocw_worldstore::stream::MemoryStore>,
        >,
    >,
);

#[cfg(not(target_arch = "wasm32"))]
fn drive_native(driver: Option<Res<NativeDriver>>, stream: Option<Res<WorldStream>>) {
    let (Some(driver), Some(stream)) = (driver, stream) else {
        return;
    };
    let mut d = driver.0.lock().expect("not poisoned");
    for _ in 0..64 {
        if !bevy::tasks::block_on(d.step(&stream.shared)) {
            break;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};
    use ocw_world::manifest::{Manifest, SignedManifest};
    use ocw_world::package::{RoadStrip, Tile};
    use ocw_world::{GraphRoute, RoadDress, RoadSurface, RouteStep};
    use ocw_worldstore::stream::{MemoryFetch, MemoryStore};

    /// A 5 × 2 tile package with a straight road along z = 500 from x 100
    /// to 4900 and a route on it.
    pub(crate) fn package() -> WorldPackage {
        let mut tiles = std::collections::BTreeMap::new();
        for x in 0..5 {
            for z in 0..2 {
                let key = TileKey { x, z };
                let class = RoadClass::Secondary;
                tiles.insert(
                    key,
                    Tile {
                        key,
                        heights: vec![450.0; 9],
                        roads: vec![RoadStrip {
                            class,
                            width_m: 6.0,
                            points: vec![[0.0, 450.0, 500.0], [1000.0, 450.0, 500.0]],
                            surface: RoadSurface::of_class(class),
                            dress: RoadDress::of_class(class, 6.0),
                            lit: ocw_world::RoadLit::Unknown,
                        }],
                    },
                );
            }
        }
        let graph = RoadGraph {
            vertices: vec![[100.0, 500.0], [4900.0, 500.0]],
            edges: vec![Edge {
                id: EdgeId(7),
                from: VertexId(0),
                to: VertexId(1),
                points: vec![[100.0, 500.0], [4900.0, 500.0]],
                class: RoadClass::Secondary,
                name: Some("Landstrasse".into()),
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
        };
        WorldPackage {
            name: "Strip".into(),
            origin: (47.14, 9.52),
            tile_size_m: 1000.0,
            samples: 3,
            tiles,
            graph: Some(graph),
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![GraphRoute {
                name: "Vaduz–Malbun".into(),
                is_loop: false,
                steps: vec![RouteStep::Edge {
                    id: EdgeId(7),
                    from_m: 0.0,
                    to_m: 4800.0,
                }],
            }],
            region: None,
            time_zone: Some("Europe/Vaduz".into()),
            look: Default::default(),
        }
    }

    /// The package published at `worlds/test/strip/2/` (unsigned).
    pub(crate) fn server() -> MemoryFetch {
        let bytes = package().to_bytes();
        let mut m = Manifest::new("test/strip", "Strip", 2, 0, "test", vec![]).unwrap();
        m.add_file("strip.ocwpkg", &bytes).unwrap();
        MemoryFetch::with([
            ("worlds/test/strip/latest".to_string(), b"2\n".to_vec()),
            (
                "worlds/test/strip/2/manifest.json".to_string(),
                SignedManifest::unsigned(&m).to_json().into_bytes(),
            ),
            ("worlds/test/strip/2/strip.ocwpkg".to_string(), bytes),
        ])
    }

    pub(crate) fn options() -> WorldOptions {
        WorldOptions {
            allow_unsigned: true,
            route: Some("vaduz-malbun".into()),
            residency: Residency {
                fetch_radius: 1,
                keep_radius: 2,
                budget_bytes: u64::MAX,
            },
            ..WorldOptions::new("test/strip")
        }
    }

    /// The driver of the native tests over `fetch` and `store`.
    pub(crate) fn driver(
        fetch: MemoryFetch,
        store: Arc<MemoryStore>,
        opts: &WorldOptions,
    ) -> NativeDriver {
        let t = std::sync::atomic::AtomicU64::new(0);
        NativeDriver(Mutex::new(
            ocw_worldstore::stream::driver::StreamDriver::new(
                fetch,
                Some(store),
                opts.stream_config(0).unwrap(),
                Box::new(move || (t.fetch_add(5, std::sync::atomic::Ordering::Relaxed) + 5) as f64),
            ),
        ))
    }

    #[test]
    fn route_names_and_rides() {
        assert_eq!(slug("Vaduz–Malbun"), "vaduz-malbun");
        assert_eq!(slug("  Rund um  Vaduz "), "rund-um-vaduz");
        let pkg = package();
        let ride = world_ride(&pkg, &options(), "Ada").unwrap();
        assert!(matches!(ride.course, Course::RouteRide { .. }));
        assert_eq!(ride.start, [100.0, 500.0]);
        assert!(ride.description.contains("4.8 km"), "{}", ride.description);
        // No such route: the error lists the package's routes.
        let e = world_ride(
            &pkg,
            &WorldOptions {
                route: Some("moon".into()),
                ..options()
            },
            "Ada",
        )
        .err()
        .unwrap();
        assert!(e.contains("Vaduz–Malbun"), "{e}");
        // A free ride starts somewhere on the graph.
        let free = world_ride(
            &pkg,
            &WorldOptions {
                free_ride: true,
                ..options()
            },
            "Ada",
        )
        .unwrap();
        assert!(matches!(free.course, Course::FreeRide { .. }));
        // No graph: an error.
        let mut no_graph = pkg;
        no_graph.graph = None;
        assert!(world_ride(&no_graph, &options(), "Ada").is_err());
    }

    #[test]
    fn page_roots_and_policy() {
        let mut o = WorldOptions::new("x");
        assert!(o.trust_store().roots.is_empty());
        o.trust_dev = true;
        let dev = o.trust_store().roots.len();
        assert!(dev >= 1);
        o.trust_roots.push([3; 32]);
        assert_eq!(o.trust_store().roots.len(), dev + 1);
        assert_eq!(o.stream_config(0).unwrap().policy, UnsignedPolicy::Refuse);
        o.allow_unsigned = true;
        assert_eq!(o.stream_config(0).unwrap().policy, UnsignedPolicy::Warn);
        o.spec = "Not An Id".into();
        assert!(o.stream_config(0).is_err());
    }
}
