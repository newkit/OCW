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
    let phone = std::env::var("OCW_PHONE").is_ok();
    let s = if phone {
        let mut opts = WorldOptions::new("memory-report");
        opts.route = route.map(str::to_string);
        let rings = crate::memory::MemoryMode::Phone
            .course_rings(crate::memory::MemoryMode::Phone.residency(Default::default()))
            .expect("a phone's");
        let parts: Vec<Vec<u8>> = chunks
            .iter()
            .filter(|(name, _)| {
                name == "sections"
                    || (name.starts_with("section ")
                        && !STREAM_SKIPPED_SECTIONS
                            .iter()
                            .any(|t| name == &format!("section {}", String::from_utf8_lossy(*t))))
            })
            .map(|(_, r)| bytes[r.clone()].to_vec())
            .collect();
        drop(sections);
        let mut kept = 0;
        let (s, _) = step("phone: base decoded, cut to the course", || {
            StreamedPackage::from_chunks_kept(header, parts, |p| {
                let k = crate::world::course_tiles(p, &opts, rings);
                kept = k.as_ref().map_or(0, |k| k.len());
                k
            })
            .expect("base")
        });
        println!("  ({kept} tiles along the course)");
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
