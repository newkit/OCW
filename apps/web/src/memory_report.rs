//! The browser app's memory per world, step by step, over real packages
//! (spec §14 Phase 10 B2, the memory per resident tile; not run by
//! default). It does natively what the page does with a streamed package
//! – the base decoded, the ride prepared, the tiles around the start
//! decoded, the renderer's package and world set made, the junctions and
//! the tile meshes built – and prints the live heap each step adds
//! ([`crate::heap`]), the base's parts and what a tile keeps once its
//! meshes went to the GPU.
//!
//! `OCW_PACKAGES=data/pilot/liechtenstein.ocwpkg:vaduz-malbun,… cargo test
//! --release -p ocw-web world_memory_report -- --ignored --nocapture
//! --test-threads 1`

use std::collections::BTreeMap;
use std::sync::Arc;

use ocw_render::package::{MeshArrays, TileLook, TileMeshes, build_tile_meshes_with};
use ocw_world::WorldSet;
use ocw_world::package::{STREAM_SKIPPED_SECTIONS, StreamedPackage, TileKey};

use crate::heap;
use crate::world::{WorldOptions, world_ride};

fn mb(b: i64) -> String {
    format!("{:.1} MB", b as f64 / 1e6)
}

/// Runs `f`, prints what it left on the heap.
fn step<T>(name: &str, f: impl FnOnce() -> T) -> (T, i64) {
    let before = heap::live() as i64;
    let peak = heap::peak() as i64;
    let v = f();
    let after = heap::live() as i64;
    // The highest mark it reached, when it raised the heap's peak (the
    // wasm memory keeps it: the start peak of v5.42).
    let top = heap::peak() as i64;
    let high = if top > peak {
        format!(
            "  (peak {} over the step's start; heap peak {})",
            mb(top - before),
            mb(top)
        )
    } else {
        String::new()
    };
    println!("  {name:<44} +{:>10}{high}", mb(after - before));
    (v, after - before)
}

/// The heap a clone of `v` takes.
fn cost<T: Clone>(name: &str, v: &T) -> i64 {
    let (c, d) = step(name, || v.clone());
    drop(c);
    d
}

/// The GPU-bound arrays of a tile's meshes: what `upload_tile` turns into
/// render-world meshes and images (freed in the main world after their
/// upload). What is left is what the tile keeps on the CPU.
fn strip_uploaded(t: &mut TileMeshes, parts: &mut BTreeMap<&'static str, usize>) -> usize {
    let mut n = 0;
    let mut take = |what: &'static str, m: &mut MeshArrays| {
        n += m.bytes();
        *parts.entry(what).or_default() += m.bytes();
        *m = MeshArrays::default();
    };
    take("terrain", &mut t.terrain);
    take("roads", &mut t.roads);
    take("overlays", &mut t.overlays);
    take("water", &mut t.water);
    take("bridges", &mut t.bridges);
    take("lamps", &mut t.lamps.poles);
    take("lamps", &mut t.lamps.heads);
    for p in t.houses.near.iter_mut() {
        take("houses near", &mut p.mesh);
    }
    for p in t.houses.mid.iter_mut() {
        take("houses mid", &mut p.mesh);
    }
    for p in t.houses.far.iter_mut() {
        take("houses far", &mut p.mesh);
    }
    for c in t.plants.chunks.iter_mut() {
        take("plants far", &mut c.far);
    }
    for c in t.obstacles.chunks.iter_mut() {
        take("obstacles", &mut c.mesh);
    }
    t.colour_map = None;
    t.splat = None;
    n
}

fn report(path: &str, route: Option<&str>) {
    println!("\n== {path} ({})", route.unwrap_or("first route"));
    let bytes = std::fs::read(path).expect("package");
    let chunks = ocw_world::manifest::package_chunks(&bytes).expect("chunks");
    let range = |name: &str| chunks.iter().find(|(n, _)| n == name).map(|c| c.1.clone());
    let header = &bytes[range("header").expect("header")];
    let mut sections = Vec::new();
    for (name, r) in &chunks {
        let skipped = STREAM_SKIPPED_SECTIONS
            .iter()
            .any(|t| name == &format!("section {}", String::from_utf8_lossy(*t)));
        if name == "sections" || (name.starts_with("section ") && !skipped) {
            sections.extend_from_slice(&bytes[r.clone()]);
        }
    }
    println!(
        "  package {}, base {} of sections fetched",
        mb(bytes.len() as i64),
        mb(sections.len() as i64)
    );
    println!("  sections (encoded):");
    for (tag, len) in ocw_world::package::section_sizes(&bytes) {
        if len > 200_000 {
            println!("    {} {}", String::from_utf8_lossy(&tag), mb(len as i64));
        }
    }
    // A phone (v5.42): the per-tile parts along the course only, cut while
    // the base is decoded from its chunks (as the browser does).
    // `OCW_PHONE=phone|saver|minimal` (any other value: phone): the rings
    // of that memory mode. The browser's base (v7.00, 10G): without the
    // road traffic's section, the chunks one at a time as they come;
    // `OCW_BASE_ALL=1` decodes TRFC too, `OCW_HOLD_CHUNKS=1` holds every
    // chunk first (the driver before v7.00).
    let phone = std::env::var("OCW_PHONE").ok();
    let s = if let Some(mode) = phone {
        let mut opts = WorldOptions::new("memory-report");
        opts.route = route.map(str::to_string);
        let mode = crate::memory::MemoryMode::parse(&mode)
            .flatten()
            .filter(|m| *m != crate::memory::MemoryMode::Normal)
            .unwrap_or(crate::memory::MemoryMode::Phone);
        let rings = mode
            .course_rings(mode.residency(Default::default()))
            .expect("a phone's");
        let skip: Vec<[u8; 4]> = if std::env::var("OCW_BASE_ALL").is_ok() {
            Vec::new()
        } else {
            crate::world::BROWSER_SKIPPED_SECTIONS.to_vec()
        };
        let wanted = |name: &str| {
            name == "sections"
                || (name.starts_with("section ")
                    && !STREAM_SKIPPED_SECTIONS
                        .iter()
                        .copied()
                        .chain(skip.iter())
                        .any(|t| name == format!("section {}", String::from_utf8_lossy(t))))
        };
        drop(sections);
        let held: Option<Vec<Vec<u8>>> = std::env::var("OCW_HOLD_CHUNKS").is_ok().then(|| {
            chunks
                .iter()
                .filter(|(name, _)| wanted(name))
                .map(|(_, r)| bytes[r.clone()].to_vec())
                .collect()
        });
        let mut kept = 0;
        let mut lines = Vec::new();
        let (s, _) = step(
            &format!("phone ({}): base decoded, cut", mode.name()),
            || {
                let mut log = |l: String| lines.push((l, heap::live(), heap::peak()));
                let mut decode = ocw_world::package::BaseDecode {
                    skip: skip.clone(),
                    log: Some(&mut log),
                };
                let keep = |p: &ocw_world::WorldPackage| {
                    let k = crate::world::course_tiles(p, &opts, rings);
                    kept = k.as_ref().map_or(0, |k| k.len());
                    k
                };
                match held {
                    Some(parts) => {
                        StreamedPackage::from_chunks_with(header, parts, keep, &mut decode)
                    }
                    // The browser's phone (10G, v7.02): every chunk pushed
                    // in slices of `world::SLICE_BYTES`, as the land use's
                    // big chunk comes (`OCW_WHOLE_CHUNKS=1`: whole chunks).
                    None => {
                        let slice = if std::env::var("OCW_WHOLE_CHUNKS").is_ok() {
                            usize::MAX
                        } else {
                            crate::world::SLICE_BYTES as usize
                        };
                        StreamedPackage::from_chunks_with(
                            header,
                            chunks
                                .iter()
                                .filter(|(name, _)| wanted(name))
                                .flat_map(|(_, r)| bytes[r.clone()].chunks(slice))
                                .map(<[u8]>::to_vec),
                            keep,
                            &mut decode,
                        )
                    }
                }
                .expect("base")
            },
        );
        for (l, live, peak) in &lines {
            println!(
                "    {l:<52} heap {} (peak {})",
                mb(*live as i64),
                mb(*peak as i64)
            );
        }
        println!("  ({kept} tiles along the course, rings {rings})");
        s
    } else {
        let (s, _) = step("base decoded (StreamedPackage)", || {
            StreamedPackage::from_parts(header, &sections).expect("base")
        });
        drop(sections);
        s
    };
    let base = s.base();
    println!("  the base's parts (a clone of each):");
    let (g, _) = step("graph", || base.graph.as_deref().cloned());
    drop(g);
    cost("landuse", &base.landuse);
    if let Some(ix) = s.land_use() {
        println!(
            "  {:<44}  {:>10} ({} tiles)",
            "landuse kept encoded",
            mb(ix.bytes() as i64),
            ix.len()
        );
    }
    cost("buildings", &base.buildings);
    cost("bridges", &base.bridges);
    cost("routes", &base.routes);
    cost("look.palette", &base.look.palette);
    cost("look.roofs", &base.look.roofs);
    cost("look.seasons", &base.look.seasons);
    cost("look.forest", &base.look.forest);
    cost("look.vegetation", &base.look.vegetation);
    cost("obstacles", &base.obstacles);
    cost("places", &base.places);
    cost("whole base (WorldPackage without tiles)", base);
    println!("  the ride's parts:");
    let g = base.graph.clone().expect("graph");
    let (net, _) = step("RoadNetwork::new (graph shared)", || {
        ocw_world::RoadNetwork::new(g)
    });
    let (obst, _) = step("WorldObstacles::from_package", || {
        ocw_world::obstacles::WorldObstacles::from_package(base)
    });
    drop((net, obst));
    let mut opts = WorldOptions::new("memory-report");
    opts.route = route.map(str::to_string);
    let (ride, _) = step("ride prepared (network, route, client)", || {
        world_ride(base, s.land_use(), &opts, "Ada Rider")
            .or_else(|e| {
                println!("  ({e}; the first route instead)");
                opts.route = None;
                world_ride(base, s.land_use(), &opts, "Ada Rider")
            })
            .expect("ride")
    });
    let size = f64::from(s.head.tile_size_m);
    let centre = TileKey::containing(ride.start, size);
    let mut tiles = BTreeMap::new();
    let (_, decoded) = step("49 tiles decoded (fetch radius 3)", || {
        for (name, r) in &chunks {
            let Some(k) = name.strip_prefix("tile ") else {
                continue;
            };
            let (x, z) = k.split_once(',').expect("x,z");
            let key = TileKey {
                x: x.parse().expect("x"),
                z: z.parse().expect("z"),
            };
            if key.ring(centre) <= 3 {
                let t = s.read_tile(&bytes[r.clone()]).expect("tile");
                tiles.insert(key, t);
            }
        }
    });
    println!(
        "  ({} tiles, {} each)",
        tiles.len(),
        mb(decoded / tiles.len().max(1) as i64)
    );
    let (pkg, _) = step("renderer's package (take_package)", || {
        Arc::new(s.package(tiles.values().cloned()))
    });
    let (set, _) = step("world set (WorldSet::new)", || {
        Arc::new(WorldSet::new("report", pkg.clone()))
    });
    cost("world set copied (Arc::make_mut)", &*set);
    let (junctions, _) = step("junctions (package_junctions)", || {
        ocw_render::junction_mesh::package_junctions(&pkg)
    });
    // The street lamps' poles of the tiles in the local server's
    // obstacles (spec 14 Phase 12 K3; the junctions shared with the
    // renderer's).
    let obstacles = match &ride.course {
        ocw_server::Course::FreeRide { net, .. } | ocw_server::Course::RouteRide { net, .. } => {
            net.obstacles().cloned()
        }
        ocw_server::Course::Route(_) => None,
    };
    // `OCW_NO_LAMPS=1`: without them (the memory before v5.64).
    if std::env::var("OCW_NO_LAMPS").is_err() {
        step("street lamp poles near the start (lamp_tiles)", || {
            crate::world::lamp_tiles(obstacles.as_deref(), &pkg, Some(ride.start))
        });
    }
    if let Some(o) = &obstacles {
        println!(
            "  ({} poles; the obstacles {})",
            o.lamp_count(),
            mb(o.approx_bytes() as i64)
        );
    }
    let day = Some(172.0);
    let mut look = TileLook::default().with_region(&pkg.look, None, day);
    if std::env::var("OCW_PHONE").is_ok() {
        look.plant_thinning = crate::memory::MemoryMode::Phone.plant_thinning();
    }
    let mut parts = BTreeMap::new();
    let mut kept = Vec::new();
    let (mut built_sum, mut kept_sum, mut gpu_sum, mut worst) = (0i64, 0i64, 0usize, 0i64);
    let mut n = 0;
    for &key in tiles.keys().filter(|k| k.ring(centre) <= 2) {
        let before = heap::live() as i64;
        let Some(mut t) = build_tile_meshes_with(&pkg, key, None, &look, junctions.as_deref())
        else {
            continue;
        };
        let built = heap::live() as i64 - before;
        worst = worst.max(built);
        gpu_sum += strip_uploaded(&mut t, &mut parts);
        let left = heap::live() as i64 - before;
        built_sum += built;
        kept_sum += left;
        n += 1;
        kept.push(t);
    }
    let n = n.max(1) as i64;
    println!(
        "  {n} tile meshes (ring 2): built {} each ({} at most), GPU arrays {} each, kept on the CPU {} each",
        mb(built_sum / n),
        mb(worst),
        mb(gpu_sum as i64 / n),
        mb(kept_sum / n)
    );
    for (what, b) in &parts {
        println!("    GPU arrays {what:<12} {} a tile", mb(*b as i64 / n));
    }
    println!("  heap now {}", mb(heap::live() as i64));
    drop((kept, junctions, set, pkg, ride, s));
}

#[test]
#[ignore = "needs world packages (OCW_PACKAGES)"]
fn world_memory_report() {
    let list = std::env::var("OCW_PACKAGES").expect("OCW_PACKAGES=path[:route],…");
    for item in list.split(',') {
        let (path, route) = match item.rsplit_once(':') {
            Some((p, r)) if !r.contains('/') => (p, Some(r)),
            _ => (item, None),
        };
        report(path, route);
    }
}
