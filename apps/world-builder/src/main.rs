//! World builder (spec §4.2): runs on the build machines and turns OSM
//! extracts and elevation data into world packages.
//!
//! ```text
//! world-builder graph [--pbf data/pilot/liechtenstein.osm.pbf] [--poly data/pilot/liechtenstein.poly | --bbox S,W,N,E] [--svg graph.svg]
//! ```

use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use ocw_worldgen::build::{Polygon, Region, build_road_graph};
use ocw_worldgen::dem::{Dem, HgtTile};
use ocw_worldgen::osm::OsmData;
use ocw_worldgen::profile::add_profiles;
use ocw_worldgen::svg::{Colouring, graph_svg_with};

#[derive(Parser)]
#[command(about = "OCW world builder (spec §4)")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the road graph of a region and print its statistics.
    Graph {
        /// OSM PBF extract.
        #[arg(long, default_value = "data/pilot/liechtenstein.osm.pbf")]
        pbf: PathBuf,
        /// Region boundary as an Osmosis/Geofabrik .poly file.
        #[arg(long, default_value = "data/pilot/liechtenstein.poly")]
        poly: PathBuf,
        /// Region as south,west,north,east in degrees instead of --poly.
        #[arg(long, value_parser = parse_bbox)]
        bbox: Option<Region>,
        /// SRTM elevation tiles (.hgt); adds road elevation profiles.
        #[arg(long = "hgt", default_values = ["data/pilot/N47E009.hgt"])]
        hgt: Vec<PathBuf>,
        /// Write an SVG map of the graph, coloured by road class.
        #[arg(long)]
        svg: Option<PathBuf>,
        /// Write an SVG map coloured by gradient.
        #[arg(long)]
        svg_grade: Option<PathBuf>,
        /// List the roads within 40 m of local x,z (diagnostics).
        #[arg(long, value_parser = parse_xz)]
        probe: Option<(f64, f64)>,
    },
    /// Build the world package of a region: road-conformed terrain and road
    /// strips in 1 km tiles (spec §4.5, §4.7).
    Package {
        #[arg(long, default_value = "data/pilot/liechtenstein.osm.pbf")]
        pbf: PathBuf,
        #[arg(long, default_value = "data/pilot/liechtenstein.poly")]
        poly: PathBuf,
        #[arg(long = "hgt", default_values = ["data/pilot/N47E009.hgt"])]
        hgt: Vec<PathBuf>,
        #[arg(long, default_value = "Liechtenstein")]
        name: String,
        /// Terrain samples per 1 km tile side (101 = 10 m grid).
        #[arg(long, default_value_t = 101)]
        samples: u32,
        /// Extra ring of terrain-only tiles around the roads.
        #[arg(long, default_value_t = 2)]
        margin_tiles: i32,
        /// Land-use cells per tile side (200 = 5 m splat map).
        #[arg(long, default_value_t = ocw_worldgen::landuse::CELLS)]
        landuse_cells: u32,
        /// Leave out land use and buildings (roads and terrain only).
        #[arg(long)]
        no_areas: bool,
        #[arg(long, default_value = "data/pilot/liechtenstein.ocwpkg")]
        out: PathBuf,
    },
    /// Validate a world package: where does the terrain show through roads?
    Check {
        #[arg(long, default_value = "data/pilot/liechtenstein.ocwpkg")]
        world: PathBuf,
    },
    /// Build a route along the road graph (preferring main roads) and write
    /// it with its elevation and the surrounding terrain (ocw-route file).
    Route {
        #[arg(long, default_value = "data/pilot/liechtenstein.osm.pbf")]
        pbf: PathBuf,
        #[arg(long, default_value = "data/pilot/liechtenstein.poly")]
        poly: PathBuf,
        #[arg(long = "hgt", default_values = ["data/pilot/N47E009.hgt"])]
        hgt: Vec<PathBuf>,
        /// Start as lat,lon (default: Vaduz).
        #[arg(long, default_value = "47.1395,9.5220", value_parser = parse_latlon)]
        from: (f64, f64),
        /// End as lat,lon (default: Malbun).
        #[arg(long, default_value = "47.1013,9.6094", value_parser = parse_latlon)]
        to: (f64, f64),
        #[arg(long, default_value = "Vaduz - Malbun")]
        name: String,
        /// Terrain margin around the route, in metres.
        #[arg(long, default_value_t = 1500.0)]
        margin: f64,
        #[arg(long, default_value = "data/pilot/vaduz-malbun.route")]
        out: PathBuf,
    },
}

fn parse_xz(s: &str) -> Result<(f64, f64), String> {
    let (a, b) = s.split_once(',').ok_or("expected x,z")?;
    Ok((
        a.trim().parse().map_err(|e| format!("{e}"))?,
        b.trim().parse().map_err(|e| format!("{e}"))?,
    ))
}

fn parse_latlon(s: &str) -> Result<(f64, f64), String> {
    let (a, b) = s.split_once(',').ok_or("expected lat,lon")?;
    let lat = a.trim().parse::<f64>().map_err(|e| e.to_string())?;
    let lon = b.trim().parse::<f64>().map_err(|e| e.to_string())?;
    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
        Ok((lat, lon))
    } else {
        Err("latitude or longitude out of range".into())
    }
}

fn load_region(poly: &PathBuf) -> Region {
    let text = std::fs::read_to_string(poly).unwrap_or_else(|e| {
        eprintln!("{}: {e} (or use --bbox)", poly.display());
        std::process::exit(1);
    });
    Region::from_polygon(Polygon::parse_poly(&text).unwrap_or_else(|e| {
        eprintln!("{}: {e}", poly.display());
        std::process::exit(1);
    }))
}

fn load_dem(hgt: &[PathBuf]) -> Dem {
    let mut dem = Dem::default();
    for path in hgt {
        match HgtTile::read(path) {
            Ok(tile) => dem.tiles.push(tile),
            Err(e) => eprintln!("{}: {e} (no elevation)", path.display()),
        }
    }
    dem
}

fn route(args: RouteArgs) {
    let region = load_region(&args.poly);
    let proj = region.projection();
    let osm = OsmData::read_pbf(&args.pbf).unwrap_or_else(|e| {
        eprintln!("{}: {e}", args.pbf.display());
        std::process::exit(1);
    });
    let (mut graph, _) = build_road_graph(&osm, &region);
    let dem = load_dem(&args.hgt);
    add_profiles(&mut graph, &dem, &proj);
    let (a, b) = (
        proj.project(args.from.0, args.from.1),
        proj.project(args.to.0, args.to.1),
    );
    let (va, vb) = (
        graph.nearest_vertex(a).expect("graph"),
        graph.nearest_vertex(b).expect("graph"),
    );
    let Some(path) = graph.shortest_path(va, vb, ocw_worldgen::route::road_cost) else {
        eprintln!("no path between the two points");
        std::process::exit(1);
    };
    let points = ocw_worldgen::route::path_points(&graph, &path);
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &points {
        min = [min[0].min(p[0]), min[1].min(p[2])];
        max = [max[0].max(p[0]), max[1].max(p[2])];
    }
    let cell = 20.0;
    let origin = [min[0] - args.margin, min[1] - args.margin];
    let nx = ((max[0] - min[0] + 2.0 * args.margin) / cell).ceil() as usize + 1;
    let nz = ((max[1] - min[1] + 2.0 * args.margin) / cell).ceil() as usize + 1;
    let mut heights = Vec::with_capacity(nx * nz);
    for k in 0..nz {
        for i in 0..nx {
            let (lat, lon) =
                proj.unproject([origin[0] + i as f64 * cell, origin[1] + k as f64 * cell]);
            heights.push(dem.elevation(lat, lon).unwrap_or(0.0) as f32);
        }
    }
    let length: f64 = path.iter().map(|&(e, _)| graph.edges[e].length_m()).sum();
    let gain: f64 = points
        .windows(2)
        .map(|w| (w[1][1] - w[0][1]).max(0.0))
        .sum();
    let names: Vec<String> = {
        let mut v: Vec<String> = Vec::new();
        for &(e, _) in &path {
            if let Some(n) = &graph.edges[e].name
                && v.last() != Some(n)
            {
                v.push(n.clone());
            }
        }
        v
    };
    let file = ocw_world::RouteFile {
        name: args.name.clone(),
        origin: proj_origin(&region),
        width_m: 6.5,
        points,
        surfaces: ocw_worldgen::route::path_surfaces(&graph, &path),
        grid: Some(ocw_world::ElevationGrid {
            origin,
            cell,
            nx,
            nz,
            heights,
        }),
    };
    if let Some(dir) = args.out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&args.out, file.to_text()).expect("write route");
    println!(
        "{}: {:.1} km, {:.0} m climbing, {} edges, {} points, terrain {nx}x{nz} at {cell} m",
        args.name,
        length / 1000.0,
        gain,
        path.len(),
        file.points.len()
    );
    println!("via: {}", names.join(" > "));
    // Steepest stretches of the final route (25 m windows).
    let mut walked = 0.0;
    let mut steep: Vec<(f64, f64, usize)> = Vec::new();
    let mut edge_at = Vec::new();
    for &(e, _) in &path {
        let n = (graph.edges[e].length_m() / 5.0).ceil() as usize;
        edge_at.extend(std::iter::repeat_n(e, n.max(1)));
    }
    for i in 0..file.points.len().saturating_sub(5) {
        let (a, b) = (file.points[i], file.points[i + 5]);
        // Along the road, not the chord (which cuts hairpins short).
        let run = file.points[i..=i + 5]
            .windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt())
            .sum::<f64>()
            .max(1e-6);
        steep.push((
            (b[1] - a[1]) / run,
            walked,
            edge_at.get(i).copied().unwrap_or(0),
        ));
        walked += ((file.points[i + 1][0] - a[0]).powi(2) + (file.points[i + 1][2] - a[2]).powi(2))
            .sqrt();
    }
    steep.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    for (g, at, e) in steep.iter().take(3) {
        let e = &graph.edges[*e];
        println!(
            "  steepest: {:+.1} % at {:.2} km ({:?} {} m, {}, way {})",
            g * 100.0,
            at / 1000.0,
            e.class,
            e.length_m().round(),
            e.name.as_deref().unwrap_or("unnamed"),
            e.osm_way
        );
    }
    println!("route file: {}", args.out.display());
}

fn proj_origin(region: &Region) -> (f64, f64) {
    region.center()
}

struct RouteArgs {
    pbf: PathBuf,
    poly: PathBuf,
    hgt: Vec<PathBuf>,
    from: (f64, f64),
    to: (f64, f64),
    name: String,
    margin: f64,
    out: PathBuf,
}

fn parse_bbox(s: &str) -> Result<Region, String> {
    let v: Vec<f64> = s
        .split(',')
        .map(|p| p.trim().parse::<f64>().map_err(|e| format!("{p}: {e}")))
        .collect::<Result<_, _>>()?;
    match v[..] {
        [min_lat, min_lon, max_lat, max_lon] if min_lat < max_lat && min_lon < max_lon => {
            Ok(Region::bbox(min_lat, min_lon, max_lat, max_lon))
        }
        _ => Err("expected south,west,north,east with south < north and west < east".into()),
    }
}

struct PackageArgs {
    pbf: PathBuf,
    poly: PathBuf,
    hgt: Vec<PathBuf>,
    name: String,
    samples: u32,
    margin_tiles: i32,
    landuse_cells: u32,
    no_areas: bool,
    out: PathBuf,
}

fn package(args: PackageArgs) {
    let PackageArgs {
        pbf,
        poly,
        hgt,
        name,
        samples,
        margin_tiles,
        landuse_cells,
        no_areas,
        out,
    } = args;
    let t = Instant::now();
    let region = load_region(&poly);
    let proj = region.projection();
    let osm = OsmData::read_pbf(&pbf).unwrap_or_else(|e| {
        eprintln!("{}: {e}", pbf.display());
        std::process::exit(1);
    });
    let (mut graph, stats) = build_road_graph(&osm, &region);
    let dem = load_dem(&hgt);
    add_profiles(&mut graph, &dem, &proj);
    let base = |x: f64, z: f64| {
        let (lat, lon) = proj.unproject([x, z]);
        dem.elevation(lat, lon).unwrap_or(0.0)
    };
    let mut pkg = ocw_worldgen::package::build_package(
        &name,
        region.center(),
        &graph,
        base,
        1000.0,
        samples,
        margin_tiles,
    );
    let terrain_s = t.elapsed().as_secs_f64();
    let mut area_report = Vec::new();
    if !no_areas {
        let ta = Instant::now();
        let data = ocw_worldgen::areas::AreaData::read_pbf(&pbf).unwrap_or_else(|e| {
            eprintln!("{}: {e}", pbf.display());
            std::process::exit(1);
        });
        let (areas, lines) = ocw_worldgen::landuse::extract(&data, &proj);
        let footprints = ocw_worldgen::buildings::footprints(&data, &proj);
        let read_s = ta.elapsed().as_secs_f64();
        let tl = Instant::now();
        let ls = ocw_worldgen::landuse::add_landuse(&mut pkg, &areas, &lines, landuse_cells);
        let landuse_s = tl.elapsed().as_secs_f64();
        let tb = Instant::now();
        let bs = ocw_worldgen::buildings::add_buildings(&mut pkg, &footprints);
        let buildings_s = tb.elapsed().as_secs_f64();
        area_report.push(format!(
            "areas: read {} areas, {} waterways, {} footprints in {read_s:.1} s",
            ls.areas, ls.water_lines, bs.footprints
        ));
        area_report.push(format!(
            "land use (LUSE, {landuse_cells} cells/tile): {}; {} vertices under water, {} with a water level; {landuse_s:.1} s",
            landuse_summary(&ls.cells),
            ls.water_vertices,
            ls.level_vertices
        ));
        area_report.push(format!(
            "buildings (BLDG): {} placed ({} gable roofs, {} with courtyards), {} outside the tiles; {buildings_s:.1} s",
            bs.placed, bs.gabled, bs.with_holes, bs.outside
        ));
    }
    let bytes = pkg.to_bytes();
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&out, &bytes).expect("write package");
    let strips: usize = pkg.tiles.values().map(|t| t.roads.len()).sum();
    println!(
        "{name}: {} tiles ({} m grid), {} road strips from {} edges ({:.0} km), {:.1} MB, {:.1} s (terrain {terrain_s:.1} s)",
        pkg.tiles.len(),
        pkg.spacing_m(),
        strips,
        stats.edges,
        stats.length_km,
        bytes.len() as f64 / 1e6,
        t.elapsed().as_secs_f64()
    );
    for line in area_report {
        println!("{line}");
    }
    print_sections(&bytes);
    let adj = graph.adjacency();
    println!(
        "road graph (format OCWPKG02): {} vertices, {} junctions, {} one-way edges, {} dead-end edges",
        graph.vertices.len(),
        adj.iter().filter(|a| a.len() > 2).count(),
        graph
            .edges
            .iter()
            .filter(|e| e.oneway != ocw_world::graph::Oneway::No)
            .count(),
        stats.dead_end_edges
    );
    println!("package: {}", out.display());
}

/// The share of each land class, largest first.
fn landuse_summary(cells: &[usize]) -> String {
    let total: usize = cells.iter().sum::<usize>().max(1);
    let mut v: Vec<(usize, usize)> = cells
        .iter()
        .copied()
        .enumerate()
        .filter(|c| c.1 > 0)
        .collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    v.iter()
        .map(|&(code, n)| {
            let class = ocw_world::landuse::LandClass::from_code(code as u8).expect("known code");
            format!("{class:?} {:.1} %", 100.0 * n as f64 / total as f64)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Prints the tagged sections of a package (tag and size).
fn print_sections(bytes: &[u8]) {
    let sections = ocw_world::package::section_sizes(bytes);
    if sections.is_empty() {
        println!("format OCWPKG01 (no sections)");
    } else {
        let list: Vec<String> = sections
            .iter()
            .map(|(tag, len)| {
                format!(
                    "{} {:.2} MB",
                    String::from_utf8_lossy(tag),
                    *len as f64 / 1e6
                )
            })
            .collect();
        println!("format OCWPKG02, sections: {}", list.join(", "));
    }
}

fn main() {
    match Cli::parse().command {
        Cmd::Check { world } => {
            let bytes = std::fs::read(&world).unwrap_or_else(|e| {
                eprintln!("{}: {e}", world.display());
                std::process::exit(1);
            });
            let pkg = ocw_world::WorldPackage::from_bytes(&bytes).unwrap_or_else(|e| {
                eprintln!("{}: {e}", world.display());
                std::process::exit(1);
            });
            print_sections(&bytes);
            if let Some(l) = &pkg.landuse {
                println!(
                    "land use: {} tiles at {} cells/side: {}; {} water-level vertices",
                    l.tiles.len(),
                    l.cells,
                    landuse_summary(&l.histogram()),
                    l.tiles.values().map(|t| t.water.len()).sum::<usize>()
                );
            } else {
                println!("land use: none (the client shows plain grass and placeholder trees)");
            }
            if let Some(b) = &pkg.buildings {
                let all: Vec<&ocw_world::buildings::Building> = b.values().flatten().collect();
                let gabled = all
                    .iter()
                    .filter(|b| matches!(b.roof, ocw_world::buildings::Roof::Gabled { .. }))
                    .count();
                let tallest = all
                    .iter()
                    .map(|b| b.eave_y - b.base_y)
                    .fold(0.0f32, f32::max);
                println!(
                    "buildings: {} in {} tiles, {gabled} with gable roofs, tallest wall {tallest:.1} m",
                    all.len(),
                    b.len()
                );
            } else {
                println!("buildings: none");
            }
            let (worst, at, above, total) = pkg.road_pokes(1.0);
            println!(
                "terrain above road surface: {above} of {total} samples ({:.3} %), worst {:.2} m at x {:.0} z {:.0}",
                100.0 * above as f64 / total.max(1) as f64,
                worst,
                at[0],
                at[1]
            );
            // Diagnostics: the terrain cell and the road points around the worst spot.
            let size = f64::from(pkg.tile_size_m);
            let key = ocw_world::package::TileKey::containing(at, size);
            if let Some(tile) = pkg.tiles.get(&key) {
                let o = key.origin(size);
                let (n, sp) = (pkg.samples as usize, pkg.spacing_m());
                let (i, k) = (
                    ((at[0] - o[0]) / sp) as usize,
                    ((at[1] - o[1]) / sp) as usize,
                );
                for (di, dk) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    println!(
                        "  vertex ({:.0},{:.0}) {:.1} m",
                        o[0] + (i + di) as f64 * sp,
                        o[1] + (k + dk) as f64 * sp,
                        tile.heights[(k + dk) * n + i + di]
                    );
                }
                for r in &tile.roads {
                    for p in &r.points {
                        let d = ((f64::from(p[0]) - at[0]).powi(2)
                            + (f64::from(p[2]) - at[1]).powi(2))
                        .sqrt();
                        if d < 15.0 {
                            println!(
                                "  road {:?} point ({:.1},{:.1}) {:.1} m, {:.1} m away",
                                r.class, p[0], p[2], p[1], d
                            );
                        }
                    }
                }
            }
        }
        Cmd::Package {
            pbf,
            poly,
            hgt,
            name,
            samples,
            margin_tiles,
            landuse_cells,
            no_areas,
            out,
        } => package(PackageArgs {
            pbf,
            poly,
            hgt,
            name,
            samples,
            margin_tiles,
            landuse_cells,
            no_areas,
            out,
        }),
        Cmd::Route {
            pbf,
            poly,
            hgt,
            from,
            to,
            name,
            margin,
            out,
        } => route(RouteArgs {
            pbf,
            poly,
            hgt,
            from,
            to,
            name,
            margin,
            out,
        }),
        Cmd::Graph {
            pbf,
            poly,
            bbox,
            hgt,
            svg,
            svg_grade,
            probe,
        } => {
            let region = match bbox {
                Some(r) => r,
                None => {
                    let text = std::fs::read_to_string(&poly).unwrap_or_else(|e| {
                        eprintln!("{}: {e} (or use --bbox)", poly.display());
                        std::process::exit(1);
                    });
                    Region::from_polygon(Polygon::parse_poly(&text).unwrap_or_else(|e| {
                        eprintln!("{}: {e}", poly.display());
                        std::process::exit(1);
                    }))
                }
            };
            let t = Instant::now();
            let osm = OsmData::read_pbf(&pbf).unwrap_or_else(|e| {
                eprintln!("{}: {e}", pbf.display());
                std::process::exit(1);
            });
            let read_s = t.elapsed().as_secs_f64();
            let (mut graph, stats) = build_road_graph(&osm, &region);
            println!(
                "read {} ({} nodes, {} highway ways) in {read_s:.2} s",
                pbf.display(),
                osm.nodes.len(),
                osm.ways.len()
            );
            println!("{stats:#?}");
            let mut dem = Dem::default();
            for path in &hgt {
                match HgtTile::read(path) {
                    Ok(tile) => dem.tiles.push(tile),
                    Err(e) => eprintln!("{}: {e} (no elevation)", path.display()),
                }
            }
            if !dem.tiles.is_empty() {
                let p = add_profiles(&mut graph, &dem, &region.projection());
                println!(
                    "elevation: {:.0}-{:.0} m, steepest 25 m {:.1} %, {} bridges/tunnels, {} edges partly outside the DEM",
                    p.min_m,
                    p.max_m,
                    p.max_grade * 100.0,
                    p.bridges_tunnels,
                    p.missing_dem
                );
                println!(
                    "suspicious segments (raw DEM much steeper than allowed): {}",
                    p.suspicious.len()
                );
                for s in p.suspicious.iter().take(8) {
                    let e = &graph.edges[s.edge];
                    println!(
                        "  {:<24} {:?} {:4.0} m  raw {:5.1} %  smoothed {:5.1} %  ends {:.0}/{:.0} m  https://www.openstreetmap.org/way/{}",
                        s.name.as_deref().unwrap_or("(unnamed)"),
                        e.class,
                        e.length_m(),
                        s.raw_grade * 100.0,
                        s.smoothed_grade * 100.0,
                        e.profile[0],
                        e.profile.last().copied().unwrap_or_default(),
                        e.osm_way
                    );
                }
                let over = graph
                    .edges
                    .iter()
                    .filter(|e| {
                        ocw_worldgen::profile::steepest(&e.profile, 1)
                            > ocw_worldgen::profile::limits(e.class).0 + 0.01
                    })
                    .count();
                println!(
                    "edges above their class limit: {over} of {}",
                    graph.edges.len()
                );
            }
            if let Some((px, pz)) = probe {
                println!("roads within 40 m of {px},{pz}:");
                for e in &graph.edges {
                    let (mut best, mut at) = (f64::INFINITY, 0.0);
                    let mut walked = 0.0;
                    for w in e.points.windows(2) {
                        let (a, b) = (w[0], w[1]);
                        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                        let t = if len > 0.0 {
                            (((px - a[0]) * (b[0] - a[0]) + (pz - a[1]) * (b[1] - a[1]))
                                / (len * len))
                                .clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let c = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                        let d = ((px - c[0]).powi(2) + (pz - c[1]).powi(2)).sqrt();
                        if d < best {
                            best = d;
                            at = walked + len * t;
                        }
                        walked += len;
                    }
                    if best <= 40.0 {
                        println!(
                            "  {:5.1} m  {:?} {:<22} bridge {} tunnel {}  road height {:.1} m  way {}\n         {:.0} m long, v{} {:.1} m -> v{} {:.1} m, steepest {:.1} %",
                            best,
                            e.class,
                            e.name.as_deref().unwrap_or("-"),
                            e.bridge,
                            e.tunnel,
                            e.height_at(at).unwrap_or(f64::NAN),
                            e.osm_way,
                            e.length_m(),
                            e.from.0,
                            e.profile.first().copied().unwrap_or(f64::NAN),
                            e.to.0,
                            e.profile.last().copied().unwrap_or(f64::NAN),
                            100.0 * ocw_worldgen::profile::steepest(&e.profile, 1)
                        );
                    }
                }
                let (lat, lon) = region.projection().unproject([px, pz]);
                println!(
                    "  DEM there: {:.1} m  ({lat:.5}, {lon:.5})",
                    dem.elevation(lat, lon).unwrap_or(f64::NAN)
                );
            }
            println!("total {:.2} s", t.elapsed().as_secs_f64());
            if let Some(path) = svg {
                let title = format!(
                    "{}: {} edges, {:.0} km, {} dead ends, {} boundary  (© OpenStreetMap contributors)",
                    pbf.file_stem().and_then(|s| s.to_str()).unwrap_or("region"),
                    stats.edges,
                    stats.length_km,
                    stats.dead_end_edges,
                    stats.boundary_edges
                );
                std::fs::write(
                    &path,
                    graph_svg_with(&graph, 1400.0, &title, Colouring::Class),
                )
                .expect("write svg");
                println!("map: {}", path.display());
            }
            if let Some(path) = svg_grade {
                let title =
                    "Gradient of the smoothed road profiles (© OpenStreetMap contributors, SRTM)";
                std::fs::write(
                    &path,
                    graph_svg_with(&graph, 1400.0, title, Colouring::Grade),
                )
                .expect("write svg");
                println!("gradient map: {}", path.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_parsing() {
        let r = parse_bbox("47.04, 9.47,47.28,9.64").unwrap();
        assert_eq!((r.min_lat, r.max_lon), (47.04, 9.64));
        assert!(parse_bbox("1,2,3").is_err());
        assert!(parse_bbox("47.3,9.47,47.28,9.64").is_err());
        assert!(parse_bbox("a,b,c,d").is_err());
        assert_eq!(parse_latlon("47.1, 9.5"), Ok((47.1, 9.5)));
        assert!(parse_latlon("95,9").is_err());
        assert!(parse_latlon("47.1").is_err());
    }
}
