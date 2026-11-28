//! Holes in the drawn road surface of the shipped worlds (spec 4.4 *Road
//! surface without holes*; bug 2026-11-28 in Andorra: "sometimes i see
//! holes in the street … so i can see through").
//!
//! Each world package's tiles are meshed as the renderer meshes them
//! (`package::tile_road_meshes` with the package's junction surfaces) and
//! every road strip the renderer draws as a strip is probed from above on
//! a 0.25 m grid inside its carriageway (`ocw_render::road_holes`). Fails
//! when large hole spots ([`LARGE_M2`]) or the hole area per km of road
//! come back (the synthetic shapes of `road_holes_shapes` fail on any
//! hole at all).
//!
//! Packages: `OCW_HOLES_WORLD=a.ocwpkg[:b.ocwpkg…]`, else the shipped
//! worlds under `data/` (`andorra`, `pilot/liechtenstein`, `tenerife`,
//! `blackforest/blackforest-north`) where present; skipped without any.
//! A CI-sized sample of tiles by default (the [`SAMPLE_TILES`] tiles with
//! the most road, plus as many spread over the package); `OCW_HOLES_FULL=1`
//! probes every tile and `OCW_HOLES_REPORT=1` only prints (the analysis
//! run). `OCW_HOLES_CSV=path` writes the spots for pictures.
//!
//! Run: `cargo test --release -p ocw-render --test road_holes -- --nocapture`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::PathBuf;

use bevy::math::Vec3;
use ocw_render::package::tile_road_meshes;
use ocw_render::road_holes::{
    HoleSample, Indexes, SurfaceIndex, inverted_area_m2, probe_strips, spots,
};
use ocw_world::WorldPackage;
use ocw_world::package::{RoadStrip, TileKey};

/// Hole spots up to this size are counted apart (m²: four samples).
const TOLERANCE_M2: f32 = 0.25;
/// A hole spot this large is plainly visible from the saddle (m²).
const LARGE_M2: f32 = 2.0;
/// At most this many [`LARGE_M2`] spots per 100 km of road (2026-11-28,
/// all tiles: before the fix Andorra 23.5, Liechtenstein 12.9, Tenerife
/// 22.6, Black Forest north 9.9; after it 0, 0.5, 0.6 and 0.6 – the rest
/// are strips a junction cluster leaves out and the edges of shaped
/// strips, spec 4.8.3 *Road surface without holes*).
const MAX_LARGE_PER_100KM: f64 = 1.0;
/// At most this much hole area per km of road (m²; before 0.8 – 1.9,
/// after 0.11 – 0.18).
const MAX_M2_PER_KM: f64 = 0.3;
/// A sample over a road drawn within this height of the strip is a step,
/// not a hole (m).
const STEP_M: f32 = 3.0;
/// Tiles probed per world in the CI-sized run.
const SAMPLE_TILES: usize = 12;

fn worlds() -> Vec<PathBuf> {
    if let Ok(v) = std::env::var("OCW_HOLES_WORLD") {
        return v.split(':').map(PathBuf::from).collect();
    }
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    [
        "andorra/andorra.ocwpkg",
        "pilot/liechtenstein.ocwpkg",
        "tenerife/tenerife.ocwpkg",
        "blackforest/blackforest-north.ocwpkg",
    ]
    .iter()
    .map(|p| data.join(p))
    .filter(|p| p.exists())
    .collect()
}

fn strip_len(p: &[[f32; 3]]) -> f32 {
    p.windows(2)
        .map(|w| Vec3::from(w[0]).distance(Vec3::from(w[1])))
        .sum()
}

/// One hole sample with what the analysis knows about its strip.
struct Found {
    h: HoleSample,
    key: TileKey,
    class: String,
    width: f32,
    /// Terrain height under the sample less the sample's height (m).
    ground_dy: Option<f32>,
}

struct Report {
    name: String,
    km: f64,
    samples: usize,
    by: BTreeMap<String, (usize, usize)>,
    found: Vec<Found>,
    inverted_m2: f32,
}

/// The tiles probed and, per probed tile, its strips drawn as strips (J7
/// paths along a road are drawn as bands of the road and left out).
fn analyse(path: &PathBuf, full: bool) -> Report {
    let bytes = std::fs::read(path).expect("package");
    let pkg = WorldPackage::from_bytes(&bytes).expect("readable package");
    drop(bytes);
    let size = f64::from(pkg.tile_size_m);
    let junctions = ocw_render::junction_mesh::PackageJunctions::new(&pkg);
    let probed: BTreeSet<TileKey> = if full {
        pkg.tiles.keys().copied().collect()
    } else {
        let mut by_road: Vec<(f32, TileKey)> = pkg
            .tiles
            .values()
            .map(|t| (t.roads.iter().map(|s| strip_len(&s.points)).sum(), t.key))
            .collect();
        by_road.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut set: BTreeSet<TileKey> = by_road.iter().take(SAMPLE_TILES).map(|x| x.1).collect();
        // And a spread over the package (mountain roads, not only towns).
        let step = (by_road.len() / SAMPLE_TILES).max(1);
        set.extend(by_road.iter().step_by(step).take(SAMPLE_TILES).map(|x| x.1));
        set
    };
    // Row by row (z), with the meshes of the rows around kept (memory).
    let mut rows: BTreeMap<i32, Vec<TileKey>> = BTreeMap::new();
    for k in &probed {
        rows.entry(k.z).or_default().push(*k);
    }
    let mut meshed: HashMap<TileKey, (SurfaceIndex, f32)> = HashMap::new();
    let mesh = |k: TileKey, meshed: &mut HashMap<TileKey, (SurfaceIndex, f32)>| {
        if meshed.contains_key(&k) {
            return;
        }
        let mut index = SurfaceIndex::new();
        let mut inv = 0.0;
        if let Some(tile) = pkg.tiles.get(&k) {
            let near = ocw_render::bridges::bridges_near(&pkg, k);
            let m = tile_road_meshes(&pkg, tile, &near, None, junctions.as_ref());
            inv = inverted_area_m2(&m.roads, &[0]);
            index.add(&m.roads);
        }
        meshed.insert(k, (index, inv));
    };
    let mut km = 0.0;
    let mut samples = 0;
    let mut found = Vec::new();
    let mut inverted_m2 = 0.0;
    for (&z, keys) in &rows {
        meshed.retain(|k, _| k.z >= z - 1);
        for k in keys {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    mesh(
                        TileKey {
                            x: k.x + dx,
                            z: k.z + dz,
                        },
                        &mut meshed,
                    );
                }
            }
            inverted_m2 += meshed[k].1;
            // The 3 × 3 tiles' triangles, as one index for the probe.
            let index = Indexes(
                (-1..=1)
                    .flat_map(|dz| (-1..=1).map(move |dx| (dx, dz)))
                    .filter_map(|(dx, dz)| {
                        meshed.get(&TileKey {
                            x: k.x + dx,
                            z: k.z + dz,
                        })
                    })
                    .map(|(i, _)| i)
                    .collect(),
            );
            let tile = &pkg.tiles[k];
            let strips: Vec<&RoadStrip> = match (junctions.as_ref(), pkg.graph.as_ref()) {
                (Some(pj), Some(g)) => {
                    let matched = pj.strip_edges(g, *k, &tile.roads);
                    tile.roads
                        .iter()
                        .zip(&matched)
                        .filter(|(_, e)| {
                            e.is_none_or(|e| !pj.set.side_paths().get(e).copied().unwrap_or(false))
                        })
                        .map(|(s, _)| s)
                        .collect()
                }
                _ => tile.roads.iter().collect(),
            };
            let strips: Vec<RoadStrip> = strips.into_iter().cloned().collect();
            km += strips
                .iter()
                .map(|s| f64::from(strip_len(&s.points)))
                .sum::<f64>()
                / 1000.0;
            let o = k.origin(size);
            let keep = |q: Vec3| {
                let (x, z) = (f64::from(q.x) - o[0], f64::from(q.z) - o[1]);
                (0.0..size).contains(&x) && (0.0..size).contains(&z)
            };
            let (n, holes) = probe_strips(&strips, &index, &keep);
            samples += n;
            for h in holes {
                let s = &strips[h.strip];
                let ground_dy = pkg
                    .surface_height(f64::from(h.at.x), f64::from(h.at.z))
                    .map(|g| g as f32 - h.at.y);
                found.push(Found {
                    h,
                    key: *k,
                    class: format!("{:?}", s.class),
                    width: s.width_m,
                    ground_dy,
                });
            }
        }
    }
    let mut by = BTreeMap::new();
    for f in &found {
        let what = format!(
            "{:?} at {:?}{}",
            f.h.probe,
            f.h.place,
            match f.h.dy {
                Some(d) if d.abs() < 5.0 => " (road drawn within 5 m)",
                Some(_) => " (road drawn farther up/down)",
                None => "",
            }
        );
        let e = by.entry(what).or_insert((0, 0));
        e.0 += 1;
        if f.ground_dy.is_none_or(|g| g < -0.5) {
            e.1 += 1;
        }
    }
    Report {
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        km,
        samples,
        by,
        found,
        inverted_m2,
    }
}

#[test]
fn shipped_worlds_have_no_holes_in_their_roads() {
    let paths = worlds();
    if paths.is_empty() {
        eprintln!("no world packages (OCW_HOLES_WORLD or data/…): skipped");
        return;
    }
    let full = std::env::var("OCW_HOLES_FULL").is_ok_and(|v| v == "1");
    let report_only = std::env::var("OCW_HOLES_REPORT").is_ok_and(|v| v == "1");
    let mut csv = std::env::var("OCW_HOLES_CSV")
        .ok()
        .map(|p| std::fs::File::create(p).expect("csv"));
    if let Some(f) = csv.as_mut() {
        writeln!(
            f,
            "world,x,y,z,samples,area_m2,probe,place,radius_m,across_m,road_dy,ground_dy,class,width_m,tile"
        )
        .unwrap();
    }
    let mut failures = Vec::new();
    for p in &paths {
        let r = analyse(p, full);
        // Holes: no drawn road within STEP_M of the strip's height there.
        // Samples over a road drawn nearer (a side road laid under a main
        // road it crosses at another height, J4) are steps, counted apart.
        let hole = |f: &Found| f.h.dy.is_none_or(|d| d.abs() > STEP_M);
        let holes: Vec<HoleSample> = r.found.iter().filter(|f| hole(f)).map(|f| f.h).collect();
        let steps = r.found.len() - holes.len();
        // Spots, with the strip index replaced by the found sample's index.
        let indexed: Vec<HoleSample> = r
            .found
            .iter()
            .enumerate()
            .filter(|(_, f)| hole(f))
            .map(|(i, f)| HoleSample { strip: i, ..f.h })
            .collect();
        let sp = spots(&indexed);
        let big: Vec<_> = sp.iter().filter(|s| s.area_m2() > TOLERANCE_M2).collect();
        let area: f32 = sp.iter().map(|s| s.area_m2()).sum();
        let big_area: f32 = big.iter().map(|s| s.area_m2()).sum();
        eprintln!(
            "{}: {:.1} km of drawn strips, {} samples, {} hole samples ({:.1} m², {:.2} m²/km), {} spots, {} above {} m² ({:.1} m², {:.2} per km); {steps} step samples; culled carriageway triangles {:.1} m² (covered or not)",
            r.name,
            r.km,
            r.samples,
            holes.len(),
            area,
            f64::from(area) / r.km.max(1e-9),
            sp.len(),
            big.len(),
            TOLERANCE_M2,
            big_area,
            big.len() as f64 / r.km.max(1e-9),
            r.inverted_m2
        );
        for (what, (n, open)) in &r.by {
            eprintln!("  {what}: {n} samples, {open} with no terrain within 0.5 m below");
        }
        let line = |s: &ocw_render::road_holes::Spot| {
            let f = &r.found[s.strip];
            format!(
                "({:.1}, {:.1}, {:.1}) {:.2} m² {:?} {:?} r {:.1} m across {:+.1} road dy {} ground dy {} {} {:.1} m tile {},{}",
                s.at.x,
                s.at.y,
                s.at.z,
                s.area_m2(),
                s.probe,
                s.place,
                s.radius_m,
                s.across,
                f.h.dy.map_or("-".into(), |d| format!("{d:+.2}")),
                f.ground_dy.map_or("-".into(), |d| format!("{d:+.2}")),
                f.class,
                f.width,
                f.key.x,
                f.key.z
            )
        };
        for s in sp.iter().take(30) {
            eprintln!("  spot {}", line(s));
        }
        if let Some(f) = csv.as_mut() {
            for s in &sp {
                let x = &r.found[s.strip];
                writeln!(
                    f,
                    "{},{:.2},{:.2},{:.2},{},{:.3},{:?},{:?},{:.1},{:.2},{},{},{},{:.1},{} {}",
                    r.name,
                    s.at.x,
                    s.at.y,
                    s.at.z,
                    s.samples,
                    s.area_m2(),
                    s.probe,
                    s.place,
                    s.radius_m,
                    s.across,
                    x.h.dy.map_or(String::new(), |d| format!("{d:.2}")),
                    x.ground_dy.map_or(String::new(), |d| format!("{d:.2}")),
                    x.class,
                    x.width,
                    x.key.x,
                    x.key.z
                )
                .unwrap();
            }
            f.flush().unwrap();
        }
        let large: Vec<_> = sp.iter().filter(|s| s.area_m2() > LARGE_M2).collect();
        let per_100km = 100.0 * large.len() as f64 / r.km.max(1e-9);
        let per_km = f64::from(area) / r.km.max(1e-9);
        if per_100km > MAX_LARGE_PER_100KM || per_km > MAX_M2_PER_KM {
            failures.push(format!(
                "{}: {:.2} hole spots above {LARGE_M2} m² per 100 km (at most {MAX_LARGE_PER_100KM}), {per_km:.2} m² of holes per km (at most {MAX_M2_PER_KM}); the largest {}",
                r.name,
                per_100km,
                big.first().map_or_else(|| "-".to_string(), |s| line(s))
            ));
        }
    }
    if !report_only {
        assert!(failures.is_empty(), "holes in the roads: {failures:#?}");
    }
}

/// The analysis' look at single spots: `OCW_HOLES_DUMP=PKG@x,z;x,z…`
/// prints the strips, shaped strips, junction surfaces and graph
/// vertices near each point (skipped without).
#[test]
fn dump_hole_spots() {
    let Ok(spec) = std::env::var("OCW_HOLES_DUMP") else {
        return;
    };
    let (path, pts) = spec.split_once('@').expect("PKG@x,z;…");
    let pkg = WorldPackage::from_bytes(&std::fs::read(path).expect("package")).expect("package");
    let size = f64::from(pkg.tile_size_m);
    let pj = ocw_render::junction_mesh::PackageJunctions::new(&pkg).expect("junctions");
    let g = pkg.graph.as_ref().expect("graph");
    let style = ocw_world::road_dress::MarkingStyle::default();
    let near = |q: &[f32; 3], c: [f64; 2], r: f64| {
        (f64::from(q[0]) - c[0]).hypot(f64::from(q[2]) - c[1]) < r
    };
    for p in pts.split(';') {
        let (x, z) = p.split_once(',').expect("x,z");
        let c = [
            x.trim().parse::<f64>().unwrap(),
            z.trim().parse::<f64>().unwrap(),
        ];
        let key = TileKey::containing(c, size);
        eprintln!("=== spot {c:?} tile {},{}", key.x, key.z);
        let tile = &pkg.tiles[&key];
        let matched = pj.strip_edges(g, key, &tile.roads);
        for (i, (s, e)) in tile.roads.iter().zip(&matched).enumerate() {
            if !s.points.iter().any(|q| near(q, c, 25.0)) {
                continue;
            }
            let side = e.is_some_and(|e| pj.set.side_paths().get(e).copied().unwrap_or(false));
            eprintln!(
                "strip {i}: {:?} {:.1} m, {} pts, edge {:?} side-path {side}, start {:?} end {:?}",
                s.class,
                s.width_m,
                s.points.len(),
                e.map(|e| (
                    g.edges[e].id,
                    g.edges[e].from,
                    g.edges[e].to,
                    g.edges[e].bridge,
                    g.edges[e].tunnel
                )),
                s.points[0],
                s.points[s.points.len() - 1]
            );
            let close: Vec<String> = s
                .points
                .iter()
                .filter(|q| near(q, c, 25.0))
                .map(|q| format!("({:.2},{:.2},{:.2})", q[0], q[1], q[2]))
                .collect();
            eprintln!("   near: {}", close.join(" "));
        }
        let t = ocw_render::junction_mesh::shape_tile(g, &pj, key, size, &tile.roads, &style);
        for (i, s) in t.strips.iter().enumerate() {
            if !s.points.iter().any(|q| near(q, c, 25.0)) {
                continue;
            }
            eprintln!(
                "shaped {i}: owner {:?} {} pts rows {} start {:?} end {:?}",
                t.owners.get(i),
                s.points.len(),
                t.rows[i].is_some(),
                s.points[0],
                s.points[s.points.len() - 1]
            );
        }
        for (i, s) in t.strips.iter().enumerate() {
            if let Some(rows) = &t.rows[i] {
                for (q, r) in s.points.iter().zip(rows) {
                    if near(q, c, 8.0) {
                        eprintln!(
                            "   row {i} at ({:.2},{:.2}): L ({:.2},{:.2},{:.2}) R ({:.2},{:.2},{:.2})",
                            q[0], q[2], r[0][0], r[0][1], r[0][2], r[1][0], r[1][1], r[1][2]
                        );
                    }
                }
            }
        }
        // The drawn triangles round the point.
        let bridges = ocw_render::bridges::bridges_near(&pkg, key);
        let m = tile_road_meshes(&pkg, tile, &bridges, None, Some(&pj));
        for t3 in m.roads.indices.chunks_exact(3) {
            let v: Vec<[f32; 3]> = t3.iter().map(|&i| m.roads.positions[i as usize]).collect();
            let (_, kind, _) = ocw_render::road_mesh::decode_b(m.roads.uvs_b[t3[0] as usize][0]);
            if kind == 0 && v.iter().any(|q| near(q, c, 6.0)) {
                let d = (v[1][0] - v[0][0]) * (v[2][2] - v[0][2])
                    - (v[1][2] - v[0][2]) * (v[2][0] - v[0][0]);
                eprintln!(
                    "   tri kind {kind} {} ({:.2},{:.2},{:.2}) ({:.2},{:.2},{:.2}) ({:.2},{:.2},{:.2})",
                    if d < 0.0 {
                        "up"
                    } else if d > 0.0 {
                        "DOWN"
                    } else {
                        "flat"
                    },
                    v[0][0],
                    v[0][1],
                    v[0][2],
                    v[1][0],
                    v[1][1],
                    v[1][2],
                    v[2][0],
                    v[2][1],
                    v[2][2]
                );
            }
        }
        for (p, _) in &t.patches {
            if (p.centre[0] - c[0]).hypot(p.centre[2] - c[1]) < 40.0 {
                eprintln!(
                    "patch v{} centre {:?} members {:?} arms {:?}",
                    p.vertex,
                    p.centre,
                    p.members,
                    p.arms
                        .iter()
                        .map(|a| (a.edge, a.half, a.mouth_m, a.centre))
                        .collect::<Vec<_>>()
                );
            }
        }
        for (v, q) in g.vertices.iter().enumerate() {
            if (q[0] - c[0]).hypot(q[1] - c[1]) < 25.0 {
                let edges: Vec<_> = g
                    .edges
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.from.0 as usize == v || e.to.0 as usize == v)
                    .map(|(i, e)| (i, e.class, e.width_m, e.bridge, e.tunnel))
                    .collect();
                eprintln!(
                    "vertex {v} at {q:?}: {} edges {edges:?} own patch {}",
                    edges.len(),
                    pj.set.own_patch(g, v).is_some()
                );
            }
        }
    }
}
