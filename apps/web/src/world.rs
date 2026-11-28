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

/// The base sections the browser neither fetches nor decodes: the road
/// traffic's rules, signals, bus lines and car parks (`TRFC`; Tenerife 3
/// MB encoded, many more decoded): the browser shows no road traffic (spec
/// 14 Phase 18 Q9 (b)), and its local server runs none (10G, the iPhone
/// that closed the page while Andorra's and Tenerife's bases were decoded,
/// 2026-11-27).
pub const BROWSER_SKIPPED_SECTIONS: [[u8; 4]; 1] = [*ocw_world::traffic::TRAFFIC_TAG];

/// The stage lines of the world loads so far (the last
/// [`STAGES_KEPT`]; the tests read them).
static STAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Stage lines kept.
pub const STAGES_KEPT: usize = 200;

/// One stage of a world's load in the page log, with the memory (spec 14
/// Phase 10, 10G *Bisecting a phone's crash*): "world load: base: section
/// LUSE (2.10 MB) decoded – wasm 212 MB, heap 160 MB (peak 171 MB)". The
/// page log (`ocw-log.js`) stores every "world load:" line at once, so a
/// phone that closes the page leaves its last stage in the next load's
/// log.
pub fn stage(what: &str) {
    let wasm = crate::wasm_memory_bytes().map_or(String::new(), |b| {
        format!("wasm {:.0} MB, ", b as f64 / 1048576.0)
    });
    let line = format!(
        "world load: {what} – {wasm}heap {:.0} MB (peak {:.0} MB)",
        crate::heap::live() as f64 / 1048576.0,
        crate::heap::peak() as f64 / 1048576.0
    );
    info!("{line}");
    if let Ok(mut s) = STAGES.lock() {
        if s.len() >= STAGES_KEPT {
            s.remove(0);
        }
        s.push(line);
    }
}

/// For how long after the start the ride's memory steps are logged as
/// stages (s) and by how much it must grow for a line (bytes).
pub const RIDE_STAGES_S: f64 = 30.0;
pub const STAGE_STEP: u64 = 8 << 20;

/// The memory the stage lines watch: the wasm memory (never shrinks),
/// natively the heap's peak.
fn memory_mark() -> u64 {
    crate::wasm_memory_bytes().unwrap_or_else(crate::heap::peak)
}

/// The stage lines so far.
pub fn stages() -> Vec<String> {
    STAGES.lock().map(|s| s.clone()).unwrap_or_default()
}

/// A phone's big land-use chunk comes in range requests of this size
/// (`StreamConfig::slice_bytes`, 10G).
pub const SLICE_BYTES: u64 = 4 << 20;

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
    /// The preview build (feature `preview`, spec §14 Phase 10 B8a): the
    /// web preview's root instead of the production roots.
    pub preview: bool,
    /// Where worlds by id are found (the page configuration's `worlds`).
    pub base: String,
    /// A phone keeps the base's per-tile parts only within this many
    /// rings of its course (`None`: the whole world; `memory::MemoryMode::
    /// course_rings`; spec 14 Phase 10 B7, v5.42).
    pub course_rings: Option<i32>,
    /// The base sections left out ([`BROWSER_SKIPPED_SECTIONS`]; the
    /// bisect switch `?base=all` loads them all, 10G).
    pub skip_sections: Vec<[u8; 4]>,
    /// `?reach=M` / `?reach=off`: how far from the rider the neighbour
    /// tiles are meshed, instead of the memory mode's
    /// (`memory::MemoryMode::near_reach_m`; `Some(None)`: the whole ring;
    /// 10G *The first tiles' GPU memory*).
    pub near_reach: Option<Option<f64>>,
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
            preview: cfg!(feature = "preview"),
            base: WORLDS_BASE.into(),
            course_rings: None,
            skip_sections: BROWSER_SKIPPED_SECTIONS.to_vec(),
            near_reach: None,
        }
    }

    /// The roots this page trusts: the production roots (none yet) – in
    /// the preview build only the web preview's root instead (B8a) –, and
    /// on dev pages the development roots and the page's extra roots.
    pub fn trust_store(&self) -> TrustStore {
        self.trust_store_with(ocw_worldstore::trust::preview_roots())
    }

    /// [`Self::trust_store`] with `preview_roots` as the preview's roots.
    pub fn trust_store_with(&self, preview_roots: Vec<RootKey>) -> TrustStore {
        let mut trust = ocw_worldstore::trust::TrustConfig {
            dev_roots: self.trust_dev,
            root_files: Vec::new(),
            roots_dir: None,
            ..Default::default()
        }
        .trust_store()
        .unwrap_or_default();
        if self.preview {
            let production: Vec<_> = ocw_worldstore::trust::PRODUCTION_ROOTS
                .iter()
                .filter_map(|j| RootKey::parse(j).ok().map(|k| k.key_id))
                .collect();
            trust.roots.retain(|r| !production.contains(&r.key_id));
            trust.roots.splice(0..0, preview_roots);
        }
        for pk in &self.trust_roots {
            trust.roots.push(RootKey {
                kind: "ocw-root-public-key".into(),
                name: "a root of the page options (development)".into(),
                key_id: key_id(pk),
                public_key: Hex(*pk),
                dev_only: true,
                user: false,
            });
        }
        trust
    }

    /// The driver's configuration.
    pub fn stream_config(&self, now_unix_s: i64) -> Result<StreamConfig, String> {
        self.stream_config_with(now_unix_s, &[])
    }

    /// The driver's configuration, trusting also the user keys `user`
    /// (the browser's trusted world keys, spec §4.7 *User keys*; they
    /// never replace a built-in root).
    pub fn stream_config_with(
        &self,
        now_unix_s: i64,
        user: &[RootKey],
    ) -> Result<StreamConfig, String> {
        let mut keys = user.to_vec();
        keys.extend(ocw_worldstore::userkeys::once_roots());
        Ok(StreamConfig {
            world: self.spec.clone(),
            source: WorldRef::parse(&self.spec, &self.base)?,
            trust: ocw_worldstore::userkeys::with_user_keys(self.trust_store(), &keys),
            policy: if self.allow_unsigned {
                UnsignedPolicy::Warn
            } else {
                UnsignedPolicy::Refuse
            },
            now_unix_s,
            ranges: RangeParams::default(),
            cache_budget: self.cache_budget,
            batch_tiles: 16,
            // A phone: the base's per-tile parts along the course only, cut
            // while the base is decoded (v5.42).
            keep: self.course_rings.map(|rings| {
                let opts = self.clone();
                ocw_worldstore::stream::driver::KeepTiles(Arc::new(move |p: &WorldPackage| {
                    let keep = course_tiles(p, &opts, rings)?;
                    info!(
                        "world memory: the base's per-tile parts kept for the {} tiles along the course",
                        keep.len()
                    );
                    Some(keep)
                }))
            }),
            skip_sections: self.skip_sections.clone(),
            log: Some(ocw_worldstore::stream::driver::StageLog(Arc::new(stage))),
            // A phone: the land use's big chunk in slices, never held whole
            // (10G; Tenerife's is 75 MB).
            slice_bytes: self.course_rings.map(|_| SLICE_BYTES),
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
/// `landuse`: a streamed package's encoded land use, for the leaves on the
/// roads (spec 4.6.2 Q6; `None`: `pkg`'s own).
pub fn world_ride(
    pkg: &WorldPackage,
    landuse: Option<&ocw_world::landuse::LandUseIndex>,
    opts: &WorldOptions,
    name: &str,
) -> Result<WorldRide, String> {
    let graph = pkg.graph.clone().ok_or_else(|| {
        format!(
            "{}: no road graph in this world package (format OCWPKG02 needed)",
            pkg.name
        )
    })?;
    // Static obstacles from the base (spec §14 Phase 12 K3: OBST, the
    // buildings, railings and tunnel walls; no tiles needed). The street
    // lamps' poles come with the tiles as they stream in (`lamp_tiles`).
    let obstacles = Some(Arc::new(
        ocw_world::obstacles::WorldObstacles::from_streamed_base(pkg),
    ));
    stage("ride: obstacles placed");
    // The damaged roads under the wheels (spec Phase 8 Q12): the base has
    // no tiles; they are added as they are streamed in.
    let road_feel = Some(Arc::new(ocw_world::road_feel::RoadFeelMap::from_package(
        pkg,
    )));
    stage("ride: road feel");
    // Where leaves lie on the roads (spec 4.6.2 Q6: wet leaves in the
    // ride): the tiles' land use decoded one at a time.
    let leaves = match landuse {
        Some(index) => ocw_world::leaves::LeafMap::from_streamed(pkg, Some(index)),
        None => ocw_world::leaves::LeafMap::from_package(pkg),
    };
    stage("ride: leaves on the roads");
    // The ground away from the roads (spec §14 Phase 13A): the base has
    // no tiles; they are added as they are streamed in (like the road feel).
    let ground = Arc::new(ocw_world::ground::GroundMap::from_package(pkg));
    stage("ride: ground map");
    // Off the road every drawn tree is an obstacle (spec 14 Phase 13 Q10
    // (a)): the trunks come from the renderer's package of resident tiles
    // (the same `Arc`, set as the tiles arrive; no copy).
    let woods = Arc::new(ocw_world::woods::Woods::empty(pkg.tile_size_m));
    let net = Arc::new(
        RoadNetwork::new(graph)
            .with_package_name(pkg.name.clone())
            .with_obstacles(obstacles)
            .with_road_feel(road_feel)
            .with_leaves(Some(Arc::new(leaves)))
            .with_ground(Some(ground))
            .with_woods(Some(woods)),
    );
    stage("ride: road network");
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

/// A free ride's course on a phone: the rings around its start (the
/// base's per-tile parts are kept there; beyond, houses, trees and
/// obstacles are left out – the terrain and the roads come with the
/// tiles).
pub const FREE_RIDE_RINGS: i32 = 8;

/// The route the options ask for (`None`: a free ride).
fn wanted_route<'a>(
    pkg: &'a WorldPackage,
    opts: &WorldOptions,
) -> Option<&'a ocw_world::GraphRoute> {
    let name = match (&opts.route, opts.free_ride) {
        (_, true) => return None,
        (Some(r), false) => r.clone(),
        (None, false) => pkg.routes.first()?.name.clone(),
    };
    pkg.routes
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(&name) || slug(&r.name) == slug(&name))
}

/// The tiles within `rings` of the ride's course: along the route (a
/// point every 200 m), or [`FREE_RIDE_RINGS`] around a free ride's start
/// (the same start [`world_ride`] takes). `None` without a road graph or
/// when the route does not fit it ([`world_ride`] says why).
pub fn course_tiles(
    pkg: &WorldPackage,
    opts: &WorldOptions,
    rings: i32,
) -> Option<BTreeSet<TileKey>> {
    // From the graph alone: a `RoadNetwork` (its junctions, adjacency)
    // took Tenerife's heap 100 – 170 MB higher for a moment while the
    // base was decoded (10G, v7.00); the ride builds its own afterwards.
    let graph = pkg.graph.as_deref()?;
    let (points, r) = match wanted_route(pkg, opts) {
        Some(route) => (route_points(graph, route, 200.0)?, rings),
        None if opts.route.is_some() && !opts.free_ride => return None,
        None => {
            let (edge, offset, _) = RoadNetwork::random_start_in(graph, 1)?;
            let p = ocw_world::network::EdgeGeometry::new(&graph.edges[edge]).point_at(offset);
            (vec![[p[0], p[2]]], rings.max(FREE_RIDE_RINGS))
        }
    };
    let size = f64::from(pkg.tile_size_m);
    let mut keep = BTreeSet::new();
    for p in points {
        let c = TileKey::containing(p, size);
        for dz in -r..=r {
            for dx in -r..=r {
                keep.insert(TileKey {
                    x: c.x + dx,
                    z: c.z + dz,
                });
            }
        }
    }
    Some(keep)
}

/// Points every `step_m` along `route` on `graph` (x, z), as
/// `RoutePlan::points` gives them, without a network (`None`: an edge of
/// the route is not in the graph).
fn route_points(
    graph: &ocw_world::graph::RoadGraph,
    route: &ocw_world::GraphRoute,
    step_m: f64,
) -> Option<Vec<[f64; 2]>> {
    let by_id: std::collections::HashMap<_, usize> = graph
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id, i))
        .collect();
    let mut out: Vec<[f64; 2]> = Vec::new();
    for s in &route.steps {
        let ocw_world::RouteStep::Edge { id, from_m, to_m } = *s else {
            continue;
        };
        let g = ocw_world::network::EdgeGeometry::new(&graph.edges[*by_id.get(&id)?]);
        let (from_m, to_m) = (from_m.clamp(0.0, g.length()), to_m.clamp(0.0, g.length()));
        let len = (to_m - from_m).abs();
        let n = (len / step_m.max(0.5)).ceil().max(1.0) as usize;
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let p = g.point_at(from_m + (to_m - from_m) * t);
            out.push([p[0], p[2]]);
        }
    }
    Some(out)
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
    /// The trust dialog was opened for the driver's question.
    asked: bool,
    /// The damaged roads the ride feels (spec Phase 8 Q12): the streamed
    /// tiles are added as they arrive.
    road_feel: Option<Arc<ocw_world::road_feel::RoadFeelMap>>,
    /// The ground off the roads (spec §14 Phase 13A): the streamed tiles
    /// are added as they arrive, like the road feel.
    ground: Option<Arc<ocw_world::ground::GroundMap>>,
    /// The ride's static obstacles (spec 14 Phase 12 K3): the street
    /// lamps' poles of the resident tiles are placed as they arrive and
    /// dropped with them.
    obstacles: Option<Arc<ocw_world::obstacles::WorldObstacles>>,
    /// The renderer's current package (shared with it), for the poles
    /// near the rider as the rider moves.
    lamp_pkg: Option<Arc<WorldPackage>>,
    /// The trees off the road (spec 14 Phase 13 Q10): the resident tiles'
    /// package is shared with it.
    woods: Option<Arc<ocw_world::woods::Woods>>,
    /// When the ride started (s) and the memory last logged as a stage
    /// since (the world-wide builds of the first frames, 10G).
    started_s: f64,
    stage_mark: u64,
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
            asked: false,
            road_feel: None,
            ground: None,
            obstacles: None,
            lamp_pkg: None,
            woods: None,
            started_s: 0.0,
            stage_mark: 0,
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
            wasm_memory.map_or(String::new(), |m| format!(
                ", wasm memory {}, {}, {}",
                mb(m),
                crate::heap::summary(),
                crate::gpu_memory::GpuMemory::now().summary()
            ))
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
    // (A phone's base came cut to its course: `StreamConfig::keep`.)
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
    let ride = world_ride(
        base.package.base(),
        base.package.land_use(),
        &stream.options,
        &stream.session.name,
    )?;
    info!("world ride: {}", ride.description);
    stage(&format!("ride prepared: {}", ride.description));
    let mut resident = ResidentTiles::new(base.package, &base.file, stream.options.residency);
    let first = resident.update(ride.start);
    stage(&format!("{} first tiles asked for", first.len()));
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

/// The tile streaming for the renderer's ring of near tiles `ring` (the
/// render distance, 10G): fetch one ring more, keep three more – the
/// page's own policy (`?…` options) as the most (ring 2: the default 3 /
/// 5; ring 1: 2 / 4).
pub fn residency_for(page: Residency, ring: i32) -> Residency {
    Residency {
        fetch_radius: page.fetch_radius.min(ring + 1),
        keep_radius: page.keep_radius.min(ring + 3),
        budget_bytes: page.budget_bytes,
    }
}

/// The streaming each frame: base → ride, arrived tiles, residency
/// around the rider, the renderer's package, the log line.
pub fn stream_world(world: &mut World) {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let Some(mut stream) = world.remove_resource::<WorldStream>() else {
        return;
    };
    // A world signed by a user key nobody here trusts (spec §4.7 *User
    // keys*): the trust dialog; its answer goes back to the driver.
    let question = match &stream.shared.lock().expect("not poisoned").consent {
        ocw_worldstore::stream::driver::Consent::Asking(r) if !stream.asked => Some((**r).clone()),
        _ => None,
    };
    if let Some(request) = question {
        stream.asked = true;
        let msg = ocw_ui::trust::ask(
            world,
            request,
            Some("world continue".into()),
            Some("world stop".into()),
            false,
        );
        info!("{msg}");
    }
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
        // The render distance's tile ring (spec §14 Phase 10, 10G): a short
        // distance fetches and keeps fewer tiles.
        let ring = world
            .get_resource::<ocw_render::package::StreamRadius>()
            .map_or(ocw_render::package::STREAM_RADIUS, |r| r.0);
        let policy = residency_for(stream.options.residency, ring);
        let centre = focus.as_ref().map(|f| f.centre);
        if let (Some(focus), Some(r)) = (focus, stream.resident.as_mut()) {
            r.policy = policy;
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
            if let Some(g) = &stream.ground {
                g.add_package_tiles(&pkg);
            }
            let pkg = Arc::new(pkg);
            stream.lamp_pkg = Some(pkg.clone());
            publish(
                world,
                pkg,
                stream.road_feel.as_deref(),
                stream.woods.as_deref(),
            );
        }
        // Poles are only hit in wild mode (also off the road): placed
        // while the own rider is wild, dropped when it is back on the road
        // (bots never leave it), so an ordinary ride pays nothing.
        let wild = world
            .get_resource::<crate::plugin::Session>()
            .is_some_and(|s| s.0.client.wild());
        match (&stream.lamp_pkg, centre) {
            (Some(pkg), Some(c)) if wild => lamp_tiles(stream.obstacles.as_deref(), pkg, Some(c)),
            _ => drop_lamp_tiles(stream.obstacles.as_deref()),
        }
    }
    {
        let s = stream.shared.lock().expect("not poisoned");
        stream.screen = s.progress.lines();
    }
    // The first seconds of the ride (the renderer's world-wide builds –
    // far terrain, colour maps, the first tiles' meshes – come in these
    // frames): a stage line each time the memory grew by STAGE_STEP.
    if stream.started && now - stream.started_s < RIDE_STAGES_S {
        let used = memory_mark();
        if used >= stream.stage_mark + STAGE_STEP {
            stream.stage_mark = used;
            stage(&format!("riding {:.1} s", now - stream.started_s));
        }
    }
    if stream.started && now - stream.last_log_s >= LOG_INTERVAL_S {
        stream.last_log_s = now;
        info!("{}", stream.status_line(crate::wasm_memory_bytes()));
        info!(
            "{}",
            crate::gpu_memory::kinds_line(&crate::gpu_memory::kinds())
        );
    }
    world.insert_resource(stream);
}

/// New resident tiles for the renderer (no re-mesh of the tiles already
/// there; the far terrain is rebuilt from them).
fn publish(
    world: &mut World,
    pkg: Arc<WorldPackage>,
    feel: Option<&ocw_world::road_feel::RoadFeelMap>,
    woods: Option<&ocw_world::woods::Woods>,
) {
    feel_tiles(feel, &pkg);
    let Some(mut pw) = world.get_resource_mut::<PackageWorld>() else {
        return;
    };
    if let Some(w) = woods {
        w.set_package(pkg.clone());
    }
    Arc::make_mut(&mut pw.set).update_tiles(0, pkg);
    let ground = pw.ground();
    let pavement = pw.pavement();
    world.insert_resource(ground);
    world.insert_resource(pavement);
}

/// The resident tiles the ride's road feel does not have yet (spec Phase 8
/// Q12): the server and the prediction share the map, so both feel them
/// from the same tick on.
fn feel_tiles(feel: Option<&ocw_world::road_feel::RoadFeelMap>, pkg: &WorldPackage) {
    let Some(feel) = feel else {
        return;
    };
    for (key, tile) in &pkg.tiles {
        if !feel.has_tile(*key) {
            feel.add_tile(tile);
        }
    }
}

/// The street lamps' poles are placed for the resident tiles within this
/// ring around the rider's tile (the 3 × 3 a phone meshes), and only
/// while the own rider is in wild mode: a pole is only hit there, and
/// placing them needs the junctions of their roads, which stay in memory
/// (v5.64: all 49 resident tiles of Liechtenstein took 14 MB more heap).
pub const LAMP_RING: i32 = 1;

/// Drops every streamed pole (the own rider is back on the road).
pub fn drop_lamp_tiles(obstacles: Option<&ocw_world::obstacles::WorldObstacles>) {
    if let Some(o) = obstacles
        && !o.streamed_lamp_tiles().is_empty()
    {
        o.sync_streamed_lamps(&[], |_| Vec::new());
    }
}

/// The street lamps' poles of the resident tiles near `centre`
/// (package metres; [`LAMP_RING`], spec 14 Phase 12 K3; `None`: all
/// resident tiles): the tiles that came are placed as the renderer places
/// their lamps (`ocw_render::street_lamps::placed_lamps`, with the
/// package's cached junctions), those that went are dropped – the local
/// server and the prediction share the obstacles, so both have them from
/// the same tick.
pub fn lamp_tiles(
    obstacles: Option<&ocw_world::obstacles::WorldObstacles>,
    pkg: &Arc<WorldPackage>,
    centre: Option<[f64; 2]>,
) {
    let Some(o) = obstacles else {
        return;
    };
    let size = f64::from(pkg.tile_size_m);
    let wanted: Vec<TileKey> = pkg
        .tiles
        .keys()
        .copied()
        .filter(|k| centre.is_none_or(|c| k.ring(TileKey::containing(c, size)) <= LAMP_RING))
        .collect();
    // The junctions only when a tile is placed (the renderer's, cached).
    let junctions = std::cell::OnceCell::new();
    let (placed, dropped) = o.sync_streamed_lamps(&wanted, |key| {
        let j = junctions.get_or_init(|| ocw_render::junction_mesh::package_junctions(pkg));
        ocw_render::street_lamps::placed_lamps(pkg, key, j.as_deref())
    });
    if placed + dropped > 0 {
        debug!(
            "world obstacles: street lamps of {placed} tiles placed, {dropped} dropped; {} poles, {} KB",
            o.lamp_count(),
            o.approx_bytes() / 1024
        );
    }
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
    stream.started_s = now;
    stream.stage_mark = memory_mark();
    stream.last_publish_s = now;
    stream.road_feel = match &ride.course {
        Course::FreeRide { net, .. } | Course::RouteRide { net, .. } => {
            net.road_feel_map().cloned()
        }
        Course::Route(_) => None,
    };
    feel_tiles(stream.road_feel.as_deref(), &pkg);
    stream.ground = match &ride.course {
        Course::FreeRide { net, .. } | Course::RouteRide { net, .. } => net.ground().cloned(),
        Course::Route(_) => None,
    };
    if let Some(g) = &stream.ground {
        g.add_package_tiles(&pkg);
    }
    stream.obstacles = match &ride.course {
        Course::FreeRide { net, .. } | Course::RouteRide { net, .. } => net.obstacles().cloned(),
        Course::Route(_) => None,
    };
    stream.woods = match &ride.course {
        Course::FreeRide { net, .. } | Course::RouteRide { net, .. } => net.woods().cloned(),
        Course::Route(_) => None,
    };
    let id = stream.identity.clone().unwrap_or_default();
    let (zone, _) = ocw_client::sky::place_zone(pkg.time_zone.as_deref(), Some(pkg.origin));
    let pkg = Arc::new(pkg);
    stream.lamp_pkg = Some(pkg.clone());
    if let Some(w) = &stream.woods {
        w.set_package(pkg.clone());
    }
    stage(&format!("first tiles here ({} tiles)", pkg.tiles.len()));
    world.insert_resource(PackageWorld {
        set: Arc::new(WorldSet::new(&id, pkg)),
        loader: None,
    });
    stage("world set made");
    // The local server, or a remote one (B3) on the same package.
    let transport = crate::plugin::MakeTransport::for_config(world, &stream.session);
    let config = SessionConfig {
        start_m: 0.0,
        ..stream.session.clone()
    };
    let session = WebSession::start(ride.course, ride.client, &config, transport);
    stage("session started");
    world.insert_resource(Session(session));
    world.insert_resource(Place(zone));
    if let Some(mut view) = world.get_resource_mut::<RideView>() {
        view.free_ride = true;
        view.riders.clear();
    }
    ocw_render::rebuild_scene(world);
    stage("scene rebuilt: riding");
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

/// The trust dialog's answer for the streamed world (`world continue |
/// stop`): the keys trusted now (the user's and those of *Load once*) go
/// to the driver, which checks the manifest again – an untrusted key asks
/// once more.
pub fn answer_trust(world: &mut World, agree: bool) -> Result<String, String> {
    use ocw_worldstore::stream::driver::Consent;
    let keys = world
        .get_resource::<ocw_ui::trust::TrustedKeys>()
        .map(|k| k.roots())
        .unwrap_or_default();
    let mut stream = world
        .get_resource_mut::<WorldStream>()
        .ok_or("no world is being loaded")?;
    stream.asked = false;
    let mut s = stream.shared.lock().expect("not poisoned");
    if agree {
        let mut all = keys;
        all.extend(ocw_worldstore::userkeys::once_roots());
        s.consent = Consent::Granted(all);
        Ok("loading the world".into())
    } else {
        s.consent = Consent::Declined;
        Ok("the world is not loaded".into())
    }
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

    /// The render distance's ring (10G) narrows the streaming, never past
    /// the page's own policy.
    #[test]
    fn the_render_distance_narrows_the_streaming() {
        let page = Residency::default();
        assert_eq!(
            residency_for(page, ocw_render::package::STREAM_RADIUS),
            page
        );
        let near = residency_for(page, 1);
        assert_eq!((near.fetch_radius, near.keep_radius), (2, 4));
        assert_eq!(near.budget_bytes, page.budget_bytes);
        let small = Residency {
            fetch_radius: 1,
            keep_radius: 2,
            ..page
        };
        assert_eq!(residency_for(small, 2), small);
    }

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
                            condition: Default::default(),
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
            graph: Some(graph.into()),
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
            obstacles: None,
            places: None,
            junctions: None,
            traffic: None,
            aero: None,
        }
    }

    /// [`package`] with its road lit (`lit=yes`) and each tile's strip
    /// where the graph's edge runs (x from the tile's west edge, the
    /// row z = 0): a row of street lamps along it in every tile.
    fn lit_package() -> WorldPackage {
        let mut pkg = package();
        for (key, t) in pkg.tiles.iter_mut() {
            if key.z != 0 {
                t.roads.clear();
            }
            let x0 = key.x as f32 * 1000.0;
            for r in &mut t.roads {
                r.lit = ocw_world::RoadLit::Yes;
                r.points = vec![[x0, 450.0, 500.0], [x0 + 1000.0, 450.0, 500.0]];
            }
        }
        pkg
    }

    /// `pkg` with only the tiles of `keys` (the streamed versions hold
    /// the resident tiles).
    fn with_tiles(pkg: &WorldPackage, keys: &[TileKey]) -> WorldPackage {
        let mut p = pkg.clone();
        p.tiles.retain(|k, _| keys.contains(k));
        p
    }

    fn net_obstacles(ride: &WorldRide) -> Arc<ocw_world::obstacles::WorldObstacles> {
        match &ride.course {
            Course::FreeRide { net, .. } | Course::RouteRide { net, .. } => {
                net.obstacles().cloned().expect("the ride has obstacles")
            }
            Course::Route(_) => panic!("a package ride"),
        }
    }

    /// The browser's obstacles have the street lamps' poles of the tiles it
    /// holds – exactly the desktop's for the same tiles (index by index,
    /// with the model files' collision proxies) – placed as the tiles come
    /// and dropped with them (spec 14 Phase 12 K3).
    #[test]
    fn the_streamed_tiles_bring_their_lamp_poles() {
        use ocw_sim::collide::props_shapes;
        use ocw_world::obstacles::WorldObstacles;
        let full = lit_package();
        let base = with_tiles(&full, &[]);
        let ride = world_ride(&base, None, &options(), "Ada").unwrap();
        let obst = net_obstacles(&ride);
        assert_eq!(obst.lamp_count(), 0, "a streamed base has no lamps");
        let same = |keys: &[TileKey]| {
            let desktop = WorldObstacles::from_package(&with_tiles(&full, keys)).unwrap();
            assert!(desktop.lamp_count() > 0);
            assert_eq!(obst.lamp_count(), desktop.lamp_count(), "{keys:?}");
            let mut tiles: Vec<TileKey> = desktop.tiles().chain(obst.tiles()).collect();
            tiles.dedup();
            for k in tiles {
                assert_eq!(obst.tile_lamp_feet(k), desktop.tile_lamp_feet(k), "{k:?}");
                assert_eq!(
                    obst.tile_index_with(k, props_shapes()),
                    desktop.tile_index_with(k, props_shapes()),
                    "{k:?}"
                );
            }
        };
        let k = |x, z| TileKey { x, z };
        let first = [k(0, 0), k(1, 0), k(0, 1), k(1, 1)];
        lamp_tiles(Some(&obst), &Arc::new(with_tiles(&full, &first)), None);
        same(&first);
        // The rider moved east: tiles 0 went, 2 came.
        let later = [k(1, 0), k(2, 0), k(1, 1), k(2, 1)];
        lamp_tiles(Some(&obst), &Arc::new(with_tiles(&full, &later)), None);
        same(&later);
        assert_eq!(obst.tile_lamp_feet(k(0, 0)), Vec::new());
        let mut held = obst.streamed_lamp_tiles();
        held.sort();
        let mut want = later.to_vec();
        want.sort();
        assert_eq!(held, want);
        // All tiles: the desktop's whole package.
        let all: Vec<TileKey> = full.tiles.keys().copied().collect();
        lamp_tiles(Some(&obst), &Arc::new(full.clone()), None);
        same(&all);
        assert_eq!(
            obst.lamp_count(),
            WorldObstacles::from_package(&full).unwrap().lamp_count()
        );
        // Near a rider in tile (3, 0): only the tiles of its ring (LAMP_RING).
        lamp_tiles(Some(&obst), &Arc::new(full.clone()), Some([3500.0, 500.0]));
        let mut held = obst.streamed_lamp_tiles();
        held.sort();
        let mut near: Vec<TileKey> = all
            .iter()
            .copied()
            .filter(|t| t.ring(k(3, 0)) <= LAMP_RING)
            .collect();
        near.sort();
        assert_eq!(held, near);
        same(&near);
    }

    /// A cached index of a tile is built again when its streamed poles
    /// come or go (the server and the prediction keep tiles for 30 s).
    #[test]
    fn the_collision_cache_sees_poles_that_came_later() {
        use ocw_sim::collide::{ObstacleKind, WorldCache};
        let full = lit_package();
        let base = with_tiles(&full, &[]);
        let ride = world_ride(&base, None, &options(), "Ada").unwrap();
        let obst = net_obstacles(&ride);
        let mut cache = WorldCache::new(Some(obst.clone()));
        let poles = |cache: &mut WorldCache, tick| {
            let mut out = Vec::new();
            cache.solids_near([0.0, 0.0], [999.0, 999.0], tick, &mut out);
            out.iter()
                .filter(|s| s.kind == ObstacleKind::StreetLamp)
                .count()
        };
        assert_eq!(poles(&mut cache, 1), 0);
        let key = TileKey { x: 0, z: 0 };
        lamp_tiles(Some(&obst), &Arc::new(with_tiles(&full, &[key])), None);
        let n = poles(&mut cache, 2);
        assert!(n > 0 && n == obst.tile_lamp_feet(key).len(), "{n}");
        lamp_tiles(Some(&obst), &Arc::new(with_tiles(&full, &[])), None);
        assert_eq!(poles(&mut cache, 3), 0);
        // Back on the road: every pole dropped.
        lamp_tiles(Some(&obst), &Arc::new(full.clone()), None);
        assert!(poles(&mut cache, 4) > 0);
        drop_lamp_tiles(Some(&obst));
        assert_eq!(obst.lamp_count(), 0);
        assert_eq!(poles(&mut cache, 5), 0);
    }

    /// In the browser's local server a rider in wild mode bumps into a
    /// streamed tile's lamp pole (`dev.lamp`: put before the nearest pole
    /// beside the road, riding slowly at it): stopped at the pole, the
    /// server counts the bump, the prediction stops there too.
    #[test]
    fn a_browser_rider_bumps_into_a_streamed_lamp_pole() {
        use crate::session::{ServerLink, WebSession};
        let full = lit_package();
        let base = with_tiles(&full, &[]);
        let ride = world_ride(&base, None, &options(), "Ada").unwrap();
        let obst = net_obstacles(&ride);
        lamp_tiles(Some(&obst), &Arc::new(full.clone()), None);
        let mut s = WebSession::ride(ride.course, ride.client, 0, 0.0, 150.0);
        let dt = 1.0 / 60.0;
        let mut now = 0.0;
        let mut fixed = 0.0;
        let mut run = |s: &mut WebSession, seconds: f64| {
            for _ in 0..(seconds / dt).round() as u32 {
                now += dt;
                fixed += dt;
                while fixed >= ocw_core::SIM_DT {
                    fixed -= ocw_core::SIM_DT;
                    s.client.predict_step();
                }
                // The steering intent (wild mode), as the app's frame sends it.
                if let Some(msg) = s.client.steer_message(now) {
                    s.send(&msg);
                }
                s.frame(dt, now);
            }
        };
        run(&mut s, 3.0);
        s.send(&ClientCore::rules_message(Some(true), None));
        s.client.set_wild(true);
        run(&mut s, 1.0);
        s.send(&ClientCore::stage_lamp_message());
        run(&mut s, 6.0);
        let ServerLink::Local(server) = &s.server else {
            panic!("the local server");
        };
        let session = server.server().session();
        let id = s.client.rider_id().unwrap();
        let st = session.rider_state(id).unwrap();
        assert!(
            session.collision_stats().bumps >= 1,
            "{:?} {st:?}",
            session.collision_stats()
        );
        assert!(session.crash(id).is_none(), "a bump, no crash");
        assert!(st.speed < 0.5, "stopped at the pole: {st:?}");
        // The prediction stops where the server does.
        let me = s.client.state();
        assert!((me.distance - st.distance).abs() < 0.5, "{me:?} {st:?}");
    }

    /// The package published at `worlds/test/strip/2/` (unsigned), with
    /// a road traffic section (the browser leaves it out, 10G).
    pub(crate) fn server() -> MemoryFetch {
        let bytes = WorldPackage {
            traffic: Some(Arc::new(ocw_world::traffic::TrafficData::default())),
            ..package()
        }
        .to_bytes();
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

    /// The browser neither fetches nor decodes the road traffic's section
    /// (spec 14 Phase 18 Q9 (b): it shows no road traffic; 10G, the
    /// iPhone's memory); `?base=all` (bisecting) loads it.
    #[test]
    fn the_browser_skips_the_traffic_section() {
        let config = options().stream_config(0).unwrap();
        assert_eq!(config.skip_sections, vec![*ocw_world::traffic::TRAFFIC_TAG]);
        assert!(config.log.is_some(), "the stages go to the page log");
        let all = WorldOptions {
            skip_sections: Vec::new(),
            ..options()
        };
        assert!(all.stream_config(0).unwrap().skip_sections.is_empty());
        // Over the in-memory server: the base comes without it.
        let d = driver(server(), Arc::new(MemoryStore::default()), &options());
        let shared = Mutex::new(StreamShared::default());
        let mut drv = d.0.lock().unwrap();
        for _ in 0..64 {
            if !bevy::tasks::block_on(drv.step(&shared)) {
                break;
            }
        }
        let s = shared.lock().unwrap();
        let base = s.base.as_ref().expect("the base");
        assert!(base.package.base().traffic.is_none());
        assert!(base.package.base().graph.is_some());
        // With `base=all` it is there.
        let d = driver(server(), Arc::new(MemoryStore::default()), &all);
        let shared = Mutex::new(StreamShared::default());
        let mut drv = d.0.lock().unwrap();
        for _ in 0..64 {
            if !bevy::tasks::block_on(drv.step(&shared)) {
                break;
            }
        }
        let s = shared.lock().unwrap();
        assert!(
            s.base
                .as_ref()
                .expect("the base")
                .package
                .base()
                .traffic
                .is_some()
        );
    }

    /// A phone's course tiles come from the graph alone (no network built
    /// while the base is decoded, 10G) and are those of the resolved route
    /// / the free ride's start on a network.
    #[test]
    fn course_tiles_without_a_network_match_the_network() {
        let pkg = package();
        let via_net = |opts: &WorldOptions, rings: i32| {
            let net = RoadNetwork::new(pkg.graph.clone().unwrap());
            let (points, r) = match wanted_route(&pkg, opts) {
                Some(route) => (route.resolve(&net).unwrap().points(&net, 200.0), rings),
                None => {
                    let (e, o, _) = net.random_start(1).unwrap();
                    let p = net.geometry(e).point_at(o);
                    (vec![[p[0], p[2]]], rings.max(FREE_RIDE_RINGS))
                }
            };
            let mut keep = BTreeSet::new();
            for p in points {
                let c = TileKey::containing(p, 1000.0);
                for dz in -r..=r {
                    for dx in -r..=r {
                        keep.insert(TileKey {
                            x: c.x + dx,
                            z: c.z + dz,
                        });
                    }
                }
            }
            keep
        };
        for rings in [0, 2, 3] {
            let route = options();
            assert_eq!(
                course_tiles(&pkg, &route, rings).unwrap(),
                via_net(&route, rings)
            );
            let free = WorldOptions {
                free_ride: true,
                ..options()
            };
            assert_eq!(
                course_tiles(&pkg, &free, rings).unwrap(),
                via_net(&free, rings)
            );
        }
        let moon = WorldOptions {
            route: Some("moon".into()),
            ..options()
        };
        assert!(course_tiles(&pkg, &moon, 2).is_none());
    }

    #[test]
    fn route_names_and_rides() {
        assert_eq!(slug("Vaduz–Malbun"), "vaduz-malbun");
        assert_eq!(slug("  Rund um  Vaduz "), "rund-um-vaduz");
        let pkg = package();
        let ride = world_ride(&pkg, None, &options(), "Ada").unwrap();
        assert!(matches!(ride.course, Course::RouteRide { .. }));
        assert_eq!(ride.start, [100.0, 500.0]);
        assert!(ride.description.contains("4.8 km"), "{}", ride.description);
        // No such route: the error lists the package's routes.
        let e = world_ride(
            &pkg,
            None,
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
            None,
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
        assert!(world_ride(&no_graph, None, &options(), "Ada").is_err());
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

    /// The preview build (B8a) on a public page: only the web preview's
    /// root; the page's trust options (`trust=dev`, `trust-root`,
    /// `allow-unsigned`) are ignored there, so packages signed by the
    /// development roots or any other root, and unsigned ones, are refused.
    /// On localhost the dev options still add their roots.
    #[test]
    fn the_preview_trusts_only_its_root_on_public_pages() {
        use ocw_world::manifest::{RootKeyPair, SigningKeyPair, VerifyError};
        use ocw_worldstore::stream::accept_manifest;
        use ocw_worldstore::trust::{dev_roots, preview_roots};

        let other = RootKeyPair::from_seed("someone else", [9; 32]);
        let q = format!(
            "world=liechtenstein&trust=dev&allow-unsigned=1&trust-root={}",
            other.public().public_key
        );
        let (c, w) = crate::parse_query(&q, false);
        assert_eq!(w.len(), 3, "{w:?}");
        let mut o = c.world.unwrap();
        o.preview = true;
        // The built-in store: exactly the preview root.
        let built_in = o.trust_store();
        assert_eq!(built_in.roots, preview_roots());
        assert!(built_in.roots.iter().all(|r| !r.dev_only));
        for d in dev_roots() {
            assert!(built_in.roots.iter().all(|r| r.key_id != d.key_id));
        }
        let config = o.stream_config(1000).unwrap();
        assert_eq!(config.policy, UnsignedPolicy::Refuse);
        assert_eq!(config.trust.roots, preview_roots());

        // With a stand-in for the preview root (its secret key is not
        // here): its packages are accepted, every other root's refused.
        let preview = RootKeyPair::from_seed("preview stand-in", [5; 32]);
        let signed_by = |root: &RootKeyPair, seed: u8| {
            let cert = root.certify(
                "signer",
                SigningKeyPair::public_key_of([seed; 32]),
                0,
                1 << 40,
            );
            let key = SigningKeyPair::new([seed; 32], cert).unwrap();
            let mut m =
                Manifest::new("liechtenstein", "Liechtenstein", 3, 500, "t", vec![]).unwrap();
            m.add_file("liechtenstein.ocwpkg", b"bytes").unwrap();
            SignedManifest::signed(&m, &key).to_json()
        };
        let trust = o.trust_store_with(vec![preview.public()]);
        let ok = accept_manifest(&signed_by(&preview, 6), &trust, config.policy, 1000).unwrap();
        assert!(ok.1.is_trusted());
        assert!(matches!(
            accept_manifest(&signed_by(&other, 7), &trust, config.policy, 1000),
            Err(VerifyError::UnknownRoot(_))
        ));
        let unsigned = SignedManifest::unsigned(&ok.0.manifest).to_json();
        assert!(matches!(
            accept_manifest(&unsigned, &trust, config.policy, 1000),
            Err(VerifyError::Unsigned)
        ));

        // A dev page (localhost) of the preview build: the dev options
        // add their roots and the policy, as in the dev build.
        let (c, w) = crate::parse_query(&q, true);
        assert!(w.is_empty(), "{w:?}");
        let mut o = c.world.unwrap();
        o.preview = true;
        let trust = o.trust_store_with(vec![preview.public()]);
        assert_eq!(trust.roots.len(), 1 + dev_roots().len() + 1);
        assert!(
            accept_manifest(&signed_by(&other, 7), &trust, UnsignedPolicy::Refuse, 1000)
                .unwrap()
                .1
                .is_trusted()
        );
        assert_eq!(o.stream_config(0).unwrap().policy, UnsignedPolicy::Warn);
        // Without the feature (the dev build) the preview root is not trusted.
        o.preview = false;
        assert!(
            o.trust_store()
                .roots
                .iter()
                .all(|r| !preview_roots().contains(r))
        );
    }
}
