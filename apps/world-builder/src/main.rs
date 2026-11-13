//! World builder (spec §4.2): runs on the build machines and turns OSM
//! extracts and elevation data into world packages.
//!
//! ```text
//! world-builder graph [--pbf data/pilot/liechtenstein.osm.pbf] [--poly data/pilot/liechtenstein.poly | --bbox S,W,N,E] [--svg graph.svg]
//! world-builder gpx --world PKG --gpx FILE [--name NAME] [--route-file FILE]   # GPX → package route
//! world-builder routes --world PKG [--from OLD_PKG] [--remove NAME] [--export-gpx]
//! world-builder route --world PKG --from LAT,LON --to LAT,LON --name NAME
//! world-builder route [--pbf ... --poly ... --bbox S,W,N,E --origin LAT,LON] --from ... --to ... --out FILE.route
//! world-builder areas --pbf FILE --at LAT,LON        # land-use diagnostics
//! world-builder package [--id europe/liechtenstein] [--sign-key KEY] [--bbox S,W,N,E] [--origin LAT,LON]
//! world-builder keys init-root --name "OCW root"     (once, then keep root.key offline)
//! world-builder keys new-signing --name "builds 2026" [--days 365]
//! world-builder sign --key KEY PKG | verify PKG | publish PKG --out DIR | diff OLD NEW
//! ```

mod dist;

use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use ocw_worldgen::build::{Polygon, Region, build_road_graph};
use ocw_worldgen::dem::{Dem, GridTile, HgtTile};
use ocw_worldgen::osm::OsmData;
use ocw_worldgen::profile::add_profiles;
use ocw_worldgen::stages::{end as stage_end, start as stage_start, time as stage};
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
        #[command(flatten)]
        src: SourceArgs,
        /// Region as south,west,north,east in degrees instead of --poly.
        #[arg(long, value_parser = parse_bbox)]
        bbox: Option<Region>,
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
        #[command(flatten)]
        src: SourceArgs,
        #[arg(long, default_value = "Liechtenstein")]
        name: String,
        /// Terrain samples per 1 km tile side (101 = 10 m grid).
        #[arg(long, default_value_t = 101)]
        samples: u32,
        /// Extra ring of terrain-only tiles around the roads.
        #[arg(long, default_value_t = 2)]
        margin_tiles: i32,
        /// Only the tiles within --margin-tiles of a road, not the whole
        /// box of the roads (islands: no tiles of open sea, spec §4.7).
        #[arg(long)]
        tiles_near_roads: bool,
        /// --tiles-near-roads plus every tile of the box with land
        /// (natural ground above 1 m), so mountains far from roads are in
        /// the package and only the open sea stays out (spec §4.7).
        #[arg(long)]
        land_tiles: bool,
        /// Land-use cells per tile side (200 = 5 m splat map).
        #[arg(long, default_value_t = ocw_worldgen::landuse::CELLS)]
        landuse_cells: u32,
        /// Leave out land use and buildings (roads and terrain only).
        #[arg(long)]
        no_areas: bool,
        /// Skip the world cleaning passes (spec §4.8): the package as
        /// before v2.39, e.g. for before/after comparisons.
        #[arg(long)]
        no_clean: bool,
        /// Only the part of the --poly region inside this box
        /// (south,west,north,east): splits a region into neighbouring
        /// packages that share the border line (spec §2.2.2).
        #[arg(long, value_parser = parse_bbox)]
        bbox: Option<Region>,
        /// Projection origin of the local metres (lat,lon; default: the
        /// region's centre). Neighbouring packages built with the same
        /// origin share their metres and tile grid exactly.
        #[arg(long, value_parser = parse_latlon)]
        origin: Option<(f64, f64)>,
        /// IANA time zone of the area (spec §14 Phase 9; default: the zone
        /// of the region at the origin, e.g. Europe/Vaduz, Europe/Berlin,
        /// Atlantic/Canary).
        #[arg(long, value_name = "ZONE")]
        time_zone: Option<String>,
        #[arg(long, default_value = "data/pilot/liechtenstein.ocwpkg")]
        out: PathBuf,
        #[command(flatten)]
        manifest: ManifestOpts,
        /// Sign the manifest with this signing key (from `keys new-signing`);
        /// without it the manifest is written unsigned.
        #[arg(long, value_name = "KEY")]
        sign_key: Option<PathBuf>,
        /// Also write the build's stage profile (time, CPU, peak memory per
        /// stage; always printed at the end) as JSON to this file.
        #[arg(long, value_name = "FILE")]
        profile: Option<PathBuf>,
    },
    /// Key management (spec §4.7): an offline root key signs online
    /// signing keys; signing keys sign package manifests.
    Keys {
        #[command(subcommand)]
        command: KeysCmd,
    },
    /// Write the package's manifest (`PKG.manifest.json`: id, version,
    /// attribution, SHA-256 of the file and its tile chunks) and sign it.
    Sign {
        package: PathBuf,
        /// Signing key file (from `keys new-signing`).
        #[arg(long, value_name = "KEY")]
        key: PathBuf,
        #[command(flatten)]
        manifest: ManifestOpts,
    },
    /// Check a package against its manifest and the signature chain;
    /// exits 1 unless intact and signed by a trusted root.
    Verify {
        package: PathBuf,
        /// Also trust this root public key file (repeatable).
        #[arg(long, value_name = "FILE")]
        trust_root: Vec<PathBuf>,
        /// Do not trust the development roots.
        #[arg(long)]
        no_dev_root: bool,
        /// Accept intact but unsigned / untrusted packages (exit 0).
        #[arg(long)]
        allow_unsigned: bool,
    },
    /// Copy a package and its manifest into DIR/<id>/<version>/, the
    /// layout clients download from (`ocw-desktop --fetch-world URL`).
    Publish {
        package: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Compare the road graphs of two builds by stable edge ID (which
    /// roads and routes re-match after a rebuild, spec §4.4).
    Diff { old: PathBuf, new: PathBuf },
    /// Import high-resolution DTM tiles (spec §4.3): XYZ text (LGL DGM1)
    /// or GeoTIFF, block-averaged to --cell and written as one GeoTIFF per
    /// input into --out (existing outputs are kept), for `--dem`.
    DemImport {
        /// CRS of the inputs: utm32, lv95, epsg:CODE (GeoTIFFs: only when
        /// they do not name one).
        #[arg(long, value_parser = ocw_worldgen::crs::Crs::parse)]
        crs: ocw_worldgen::crs::Crs,
        /// Output cell size in metres (a multiple of the input's).
        #[arg(long, default_value_t = 2.0)]
        cell: f64,
        /// Tile size in metres: partial tiles are padded with voids to it.
        #[arg(long, default_value_t = 1000.0)]
        tile: f64,
        #[arg(long)]
        out: PathBuf,
        /// Input files (.xyz, .tif).
        files: Vec<PathBuf>,
    },
    /// Print the grid tiles (lower-left corner in km, `E_N`) that cover a
    /// latitude / longitude box, for scripted DTM downloads.
    DemTiles {
        #[arg(long, value_parser = ocw_worldgen::crs::Crs::parse)]
        crs: ocw_worldgen::crs::Crs,
        #[arg(long, value_parser = parse_bbox)]
        bbox: Region,
        /// Tile size in metres.
        #[arg(long, default_value_t = 1000.0)]
        size: f64,
        /// Lattice offset of the tile corners (east,north in metres; LGL's
        /// 2 km tiles start at odd kilometres east: 1000,0).
        #[arg(long, default_value = "0,0", value_parser = parse_xz)]
        phase: (f64, f64),
    },
    /// Print the imagery cells (lower-left corner in km, `E_N`) of a
    /// projected CRS that cover a package's tiles: what
    /// `scripts/fetch-imagery.sh` downloads (spec §14 Phase 8 T3).
    ImageryCells {
        #[arg(long)]
        world: PathBuf,
        #[arg(long, value_parser = ocw_worldgen::crs::Crs::parse)]
        crs: ocw_worldgen::crs::Crs,
        /// Cell size in metres.
        #[arg(long, default_value_t = 1000.0)]
        cell: f64,
        /// Extra margin around each tile (m).
        #[arg(long, default_value_t = 50.0)]
        margin: f64,
    },
    /// Region calibration (spec §14 Phase 8 T3): measure the regional
    /// palette, the roof colour of every building and the seasonal tint
    /// maps in the imagery of `scripts/fetch-imagery.sh` and store them in
    /// the package (sections PALT, ROOF, SEAS); the manifest is renewed
    /// with the imagery's notices.
    Calibrate {
        /// The package (rewritten in place unless --out is given).
        #[arg(long)]
        world: PathBuf,
        /// Imagery folder (e.g. data/pilot/imagery): one sub-folder with an
        /// index.json per source, orthophotos ranked by folder name.
        #[arg(long)]
        imagery: PathBuf,
        /// OSM extracts for `roof:colour` (it wins over the orthophoto).
        #[arg(long)]
        pbf: Vec<PathBuf>,
        /// Cells per tile side of the seasonal tint maps (50 = 20 m, 100 = 10 m).
        #[arg(long, default_value_t = 50)]
        season_cells: u32,
        /// Write the calibrated package here instead.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Sign the renewed manifest with this signing key.
        #[arg(long, value_name = "KEY")]
        sign_key: Option<PathBuf>,
    },
    /// Compare the DTM grids (--dem) with SRTM (--hgt) on a lattice inside
    /// a box: where the DTM covers, how much they differ.
    DemDiff {
        #[command(flatten)]
        src: SourceArgs,
        #[arg(long, value_parser = parse_bbox)]
        bbox: Region,
        /// Lattice spacing in metres.
        #[arg(long, default_value_t = 100.0)]
        step: f64,
    },
    /// Diagnostics: the OSM areas (land use, water, buildings) that contain
    /// a point and the waterways near it, with their land class.
    Areas {
        #[arg(long, default_value = "data/pilot/liechtenstein.osm.pbf")]
        pbf: PathBuf,
        /// The point as lat,lon.
        #[arg(long, value_parser = parse_latlon)]
        at: (f64, f64),
        /// Waterways within this many metres are listed.
        #[arg(long, default_value_t = 30.0)]
        radius: f64,
    },
    /// Validate a world package: where does the terrain show through roads?
    Check {
        #[arg(long, default_value = "data/pilot/liechtenstein.ocwpkg")]
        world: PathBuf,
        /// A summit that must be in the package: lat,lon,metres. The
        /// highest terrain within 150 m must be within 50 m of it, else
        /// the check exits 1 (repeatable; spec §4.7 "Tile cover").
        #[arg(long, value_parser = parse_peak)]
        peak: Vec<(f64, f64, f64)>,
    },
    /// World lint (spec §4.8.1): steps and kinks in roads, floating and
    /// buried roads, junction steps, near-miss ends, rivers flowing uphill,
    /// tilted lakes, floating buildings, land-use slivers.
    Lint {
        #[arg(long, default_value = "data/pilot/liechtenstein.ocwpkg")]
        world: PathBuf,
        /// The OSM extract(s) the package was built from: adds the river
        /// and lake checks.
        #[arg(long)]
        pbf: Vec<PathBuf>,
    },
    /// Build a route along the road graph (preferring main roads) and write
    /// it with its elevation and the surrounding terrain (ocw-route file).
    Route {
        #[command(flatten)]
        src: SourceArgs,
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
        /// Only the part of the --poly region inside this box, as for
        /// `package --bbox`: give the package's --bbox (and --origin), so
        /// the route file is in the package's metres with its road heights.
        #[arg(long, value_parser = parse_bbox)]
        bbox: Option<Region>,
        /// Projection origin (lat,lon), as for `package --origin`.
        #[arg(long, value_parser = parse_latlon)]
        origin: Option<(f64, f64)>,
        /// Skip the world cleaning passes, as for `package --no-clean`.
        #[arg(long)]
        no_clean: bool,
        /// Instead of an ocw-route file: add the route (by stable edge IDs)
        /// to this world package's routes (spec §2.7), on its road graph.
        #[arg(long, value_name = "PKG")]
        world: Option<PathBuf>,
    },
    /// Import a GPX track or route (spec §2.7): match it onto the package's
    /// road graph and store it in the package (section ROUT), optionally
    /// also as an `ocw-graph-route` file. Problems are reported.
    Gpx {
        /// The world package (version 2, with a road graph).
        #[arg(long, default_value = "data/pilot/liechtenstein.ocwpkg")]
        world: PathBuf,
        #[arg(long)]
        gpx: PathBuf,
        /// Route name (default: the GPX track's name, or the file name).
        #[arg(long)]
        name: Option<String>,
        /// Write the package here instead of updating --world.
        #[arg(long, value_name = "PKG")]
        out: Option<PathBuf>,
        /// Also write the route as a standalone `ocw-graph-route` file.
        #[arg(long, value_name = "PATH")]
        route_file: Option<PathBuf>,
        /// Only report, write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// List a package's routes and check them against its road graph; with
    /// --from, carry the routes of an older build over (by stable edge ID,
    /// re-matching the broken ones, spec §4.4).
    Routes {
        #[arg(long, default_value = "data/pilot/liechtenstein.ocwpkg")]
        world: PathBuf,
        /// An older build of the package whose routes are carried over.
        #[arg(long, value_name = "PKG")]
        from: Option<PathBuf>,
        /// Remove the route with this name.
        #[arg(long, value_name = "NAME")]
        remove: Option<String>,
        /// Write a GPX file of each route next to the package (for checking
        /// in other tools; GPX elevation from the road profile).
        #[arg(long)]
        export_gpx: bool,
    },
}

fn read_package(path: &std::path::Path) -> ocw_world::WorldPackage {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        std::process::exit(1);
    });
    ocw_world::WorldPackage::from_bytes(&bytes).unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        std::process::exit(1);
    })
}

fn write_package(pkg: &ocw_world::WorldPackage, path: &std::path::Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, pkg.to_bytes()).unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        std::process::exit(1);
    });
    // Its manifest must list the new hash, or clients refuse the package.
    dist::refresh_manifest(path);
}

fn package_network(
    pkg: &ocw_world::WorldPackage,
    path: &std::path::Path,
) -> ocw_world::RoadNetwork {
    let graph = pkg.graph.clone().unwrap_or_else(|| {
        eprintln!(
            "{}: no road graph (rebuild it with `world-builder package`)",
            path.display()
        );
        std::process::exit(1);
    });
    ocw_world::RoadNetwork::new(graph).with_package_name(pkg.name.clone())
}

/// `world-builder gpx`: match a GPX file and store the route.
fn gpx(
    world: PathBuf,
    gpx: PathBuf,
    name: Option<String>,
    out: Option<PathBuf>,
    route_file: Option<PathBuf>,
    dry_run: bool,
) {
    let mut pkg = read_package(&world);
    let net = package_network(&pkg, &world);
    let text = std::fs::read_to_string(&gpx).unwrap_or_else(|e| {
        eprintln!("{}: {e}", gpx.display());
        std::process::exit(1);
    });
    let track = ocw_world::gpx::Gpx::parse(&text).unwrap_or_else(|e| {
        eprintln!("{}: {e}", gpx.display());
        std::process::exit(1);
    });
    let name = name.or_else(|| track.name.clone()).unwrap_or_else(|| {
        gpx.file_stem()
            .map_or("GPX".into(), |s| s.to_string_lossy().into_owned())
    });
    let (s, w, n, e) = track.bounds();
    println!(
        "{}: {} points in {} part(s), {:.2} km, {s:.4},{w:.4} - {n:.4},{e:.4}",
        gpx.display(),
        track.points.len(),
        track.parts.max(1),
        track.length_m() / 1000.0
    );
    let t = Instant::now();
    let report = ocw_world::matching::match_gpx(
        &net,
        &track,
        pkg.origin,
        &name,
        &ocw_world::matching::MatchParams::default(),
    )
    .unwrap_or_else(|e| {
        eprintln!("{}: {e}", gpx.display());
        std::process::exit(1);
    });
    println!("{} ({:.2} s)", report.summary(), t.elapsed().as_secs_f64());
    let names: Vec<String> = route_streets(&net, &report.plan);
    println!("via: {}", names.join(" > "));
    if dry_run {
        return;
    }
    let stored = ocw_world::GraphRoute::from_plan(&report.plan, &net);
    if let Some(path) = &route_file {
        std::fs::write(path, stored.to_text(Some((&pkg.name, net.fingerprint()))))
            .expect("write route file");
        println!("route file: {}", path.display());
    }
    pkg.put_route(stored);
    let out = out.unwrap_or(world);
    write_package(&pkg, &out);
    println!(
        "package: {} ({} routes: {})",
        out.display(),
        pkg.routes.len(),
        pkg.routes
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Street names along a route, without repeats.
fn route_streets(net: &ocw_world::RoadNetwork, plan: &ocw_world::RoutePlan) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for s in &plan.steps {
        if let ocw_world::Segment::Edge { edge, .. } = s
            && let Some(n) = &net.edge(*edge).name
            && v.last() != Some(n)
        {
            v.push(n.clone());
        }
    }
    v
}

/// `world-builder routes`: list, check, carry over, remove, export.
fn routes(world: PathBuf, from: Option<PathBuf>, remove: Option<String>, export_gpx: bool) {
    let mut pkg = read_package(&world);
    let net = package_network(&pkg, &world);
    let mut changed = false;
    if let Some(old_path) = &from {
        let old_pkg = read_package(old_path);
        let old_net = package_network(&old_pkg, old_path);
        let mut broken = 0;
        for route in &old_pkg.routes {
            match ocw_world::matching::carry_over(route, &old_net, old_pkg.origin, &net, pkg.origin)
            {
                Ok(ocw_world::matching::CarriedOver::Kept(_)) => {
                    println!("{}: kept (all stable edge IDs fit)", route.name);
                    pkg.put_route(route.clone());
                }
                Ok(ocw_world::matching::CarriedOver::Rematched { why, report }) => {
                    println!(
                        "{}: BROKEN ({why}), re-matched: {}",
                        route.name,
                        report.summary()
                    );
                    pkg.put_route(ocw_world::GraphRoute::from_plan(&report.plan, &net));
                }
                Err(e) => {
                    broken += 1;
                    println!("{}: BROKEN, not carried over: {e}", route.name);
                }
            }
        }
        changed = true;
        if broken > 0 {
            eprintln!("{broken} route(s) could not be carried over; redo them (world-builder gpx)");
        }
    }
    if let Some(name) = &remove {
        let before = pkg.routes.len();
        pkg.routes.retain(|r| !r.name.eq_ignore_ascii_case(name));
        changed |= pkg.routes.len() != before;
        if pkg.routes.len() == before {
            eprintln!("no route {name:?}");
        }
    }
    let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
    for route in &pkg.routes {
        match route.resolve(&net) {
            Ok(plan) => {
                println!(
                    "{}: {:.2} km, {} steps{}, ok - via {}",
                    route.name,
                    plan.length_m() / 1000.0,
                    plan.steps.len(),
                    if plan.is_loop { ", loop" } else { "" },
                    route_streets(&net, &plan).join(" > ")
                );
                if export_gpx {
                    let pts: Vec<ocw_world::gpx::GpxPoint> = plan
                        .points(&net, 10.0)
                        .into_iter()
                        .map(|p| {
                            let (lat, lon) = proj.unproject(p);
                            ocw_world::gpx::GpxPoint {
                                lat,
                                lon,
                                ele: None,
                            }
                        })
                        .collect();
                    let file = world.with_file_name(format!(
                        "{}.gpx",
                        route.name.replace(['/', '\\', ' '], "_")
                    ));
                    std::fs::write(&file, ocw_world::gpx::Gpx::to_text(&route.name, &pts))
                        .expect("write gpx");
                    println!("  gpx: {}", file.display());
                }
            }
            Err(e) => println!("{}: BROKEN: {e}", route.name),
        }
    }
    if pkg.routes.is_empty() {
        println!("{}: no routes", world.display());
    }
    if changed {
        write_package(&pkg, &world);
        println!("package: {}", world.display());
    }
}

/// `world-builder route --world PKG`: the shortest path (preferring main
/// roads) between two points on the package's graph, stored as a route.
fn package_route(world: PathBuf, from: (f64, f64), to: (f64, f64), name: String) {
    let mut pkg = read_package(&world);
    let net = package_network(&pkg, &world);
    let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let graph = net.graph();
    let (va, vb) = (
        graph
            .nearest_vertex(proj.project(from.0, from.1))
            .expect("graph"),
        graph
            .nearest_vertex(proj.project(to.0, to.1))
            .expect("graph"),
    );
    let Some(path) = graph.shortest_path(va, vb, ocw_worldgen::route::road_cost) else {
        eprintln!("no path between the two points");
        std::process::exit(1);
    };
    let steps: Vec<ocw_world::Segment> = path
        .iter()
        .map(|&(e, f)| ocw_world::Segment::whole(&net, e, f))
        .collect();
    let plan = ocw_world::RoutePlan::new(&net, name.clone(), steps, false).unwrap_or_else(|e| {
        eprintln!("{name}: {e}");
        std::process::exit(1);
    });
    println!(
        "{name}: {:.1} km, {} edges, via {}",
        plan.length_m() / 1000.0,
        plan.steps.len(),
        route_streets(&net, &plan).join(" > ")
    );
    pkg.put_route(ocw_world::GraphRoute::from_plan(&plan, &net));
    write_package(&pkg, &world);
    println!("package: {} ({} routes)", world.display(), pkg.routes.len());
}

#[derive(clap::Args)]
struct ManifestOpts {
    /// Package id, e.g. europe/liechtenstein (default: from the existing
    /// manifest, else from the name).
    #[arg(long)]
    id: Option<String>,
    /// Build number (default: the existing manifest's, +1 if the package changed).
    #[arg(long = "pkg-version", value_name = "N")]
    version: Option<u64>,
    /// Attribution of the elevation data, repeatable: `srtm`, `lgl-dgm1`,
    /// `swissalti3d`, `ign-mdt05` or a free notice (default: SRTM).
    #[arg(long, value_name = "TEXT")]
    dem_attribution: Vec<String>,
}

/// The source data of a build: OSM extracts, region boundary, elevation.
#[derive(clap::Args, Clone, Debug)]
struct SourceArgs {
    /// OSM PBF extract; repeat to merge neighbouring extracts of the same
    /// day (e.g. two Geofabrik Regierungsbezirke).
    #[arg(long, default_values = ["data/pilot/liechtenstein.osm.pbf"])]
    pbf: Vec<PathBuf>,
    /// Region boundary as an Osmosis/Geofabrik .poly file; repeat to join
    /// several.
    #[arg(long, default_values = ["data/pilot/liechtenstein.poly"])]
    poly: Vec<PathBuf>,
    /// SRTM elevation tiles (.hgt): the fallback outside the --dem grids.
    #[arg(long = "hgt", default_values = ["data/pilot/N47E009.hgt"])]
    hgt: Vec<PathBuf>,
    /// High-resolution DTM (spec §4.3): GeoTIFF tiles or folders of them
    /// (e.g. from `dem-import`); repeat, best first. Only tiles near the
    /// region are read.
    #[arg(long = "dem")]
    dem: Vec<PathBuf>,
    /// Average the --dem grids to this cell size (m) when loading, to save
    /// memory (default: as stored).
    #[arg(long, value_name = "M")]
    dem_cell: Option<f64>,
}

#[derive(Subcommand)]
enum KeysCmd {
    /// Create the root key pair (DIR/root.key, DIR/root.pub.json).
    InitRoot {
        #[arg(long)]
        name: String,
        /// Key directory (default: $OCW_KEY_DIR or ~/.config/ocw/keys).
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Create a signing key certified by the root key.
    NewSigning {
        #[arg(long)]
        name: String,
        /// Root secret key (default: <key dir>/root.key).
        #[arg(long)]
        root: Option<PathBuf>,
        /// Validity in days.
        #[arg(long, default_value_t = 365)]
        days: u32,
        /// Output file (default: next to the root key, <name>.key).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

impl ManifestOpts {
    fn args(&self, name: Option<String>) -> dist::ManifestArgs {
        dist::ManifestArgs {
            id: self.id.clone(),
            name,
            version: self.version,
            dem_attribution: self.dem_attribution.clone(),
        }
    }
}

fn parse_xz(s: &str) -> Result<(f64, f64), String> {
    let (a, b) = s.split_once(',').ok_or("expected x,z")?;
    Ok((
        a.trim().parse().map_err(|e| format!("{e}"))?,
        b.trim().parse().map_err(|e| format!("{e}"))?,
    ))
}

fn parse_peak(s: &str) -> Result<(f64, f64, f64), String> {
    let (ll, h) = s.rsplit_once(',').ok_or("expected lat,lon,metres")?;
    let (lat, lon) = parse_latlon(ll)?;
    Ok((lat, lon, h.trim().parse().map_err(|e| format!("{h}: {e}"))?))
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

fn load_region(polys: &[PathBuf]) -> Region {
    let mut all = Polygon::default();
    for poly in polys {
        let text = std::fs::read_to_string(poly).unwrap_or_else(|e| {
            eprintln!("{}: {e} (or use --bbox)", poly.display());
            std::process::exit(1);
        });
        let p = Polygon::parse_poly(&text).unwrap_or_else(|e| {
            eprintln!("{}: {e}", poly.display());
            std::process::exit(1);
        });
        all.outer.extend(p.outer);
        all.holes.extend(p.holes);
    }
    Region::from_polygon(all)
}

/// How far around a region (degrees, about 5 km) OSM ways and DTM tiles
/// are kept when reading: the context roads, the margin tiles and the
/// feathering all lie within it.
const LOAD_MARGIN_DEG: f64 = 0.05;

/// The box around `r` that is read.
fn load_box(r: &Region) -> (f64, f64, f64, f64) {
    (
        r.min_lat - LOAD_MARGIN_DEG,
        r.min_lon - LOAD_MARGIN_DEG,
        r.max_lat + LOAD_MARGIN_DEG,
        r.max_lon + LOAD_MARGIN_DEG,
    )
}

/// The highway ways of the extracts (merged) that come near `near`.
fn read_osm(pbfs: &[PathBuf], near: &Region) -> OsmData {
    let t = Instant::now();
    let mut osm = stage("read PBF highways", || {
        OsmData::read_pbfs(pbfs).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        })
    });
    let all = osm.ways.len();
    stage("retain near", || osm.retain_near(load_box(near)));
    println!(
        "osm: {} of {all} highway ways near the region from {} extract(s), {:.1} s",
        osm.ways.len(),
        pbfs.len(),
        t.elapsed().as_secs_f64()
    );
    osm
}

/// Reads one DTM GeoTIFF if it overlaps the box (lat/lon), averaged to
/// `cell` if given.
fn read_dem_tile(
    path: &std::path::Path,
    (s, w, n, e): (f64, f64, f64, f64),
    cell: Option<f64>,
) -> Result<Option<GridTile>, String> {
    let err = |e: ocw_worldgen::dem::DemError| format!("{}: {e}", path.display());
    let head = GridTile::peek_geotiff(path, None).map_err(err)?;
    let b = head.crs.bounds_of(s, w, n, e);
    let tb = head.bounds();
    if tb[2] < b[0] || tb[0] > b[2] || tb[3] < b[1] || tb[1] > b[3] {
        return Ok(None);
    }
    let mut tile = GridTile::read_geotiff(path, None).map_err(err)?;
    if let Some(c) = cell {
        let f = (c / tile.cell).round();
        if (f * tile.cell - c).abs() > 1e-6 || f < 1.0 {
            return Err(format!(
                "{}: --dem-cell {c} is not a multiple of its {} m",
                path.display(),
                tile.cell
            ));
        }
        tile = tile.resampled(f as usize).map_err(err)?;
    }
    Ok(Some(tile))
}

/// The GeoTIFF files of `--dem` (folders: their `.tif` files, sorted).
fn dem_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(p)
                .map(|d| d.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            v.retain(|f| {
                f.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                    x.eq_ignore_ascii_case("tif") || x.eq_ignore_ascii_case("tiff")
                })
            });
            v.sort();
            if v.is_empty() {
                eprintln!("{}: no GeoTIFF files", p.display());
            }
            files.extend(v);
        } else {
            files.push(p.clone());
        }
    }
    files
}

/// The elevation sources (spec §4.3): the `--dem` grids near the region
/// (read in parallel), SRTM as the fallback.
fn load_dem(src: &SourceArgs, near: &Region) -> Dem {
    let t = Instant::now();
    let mut dem = Dem::default();
    stage("SRTM tiles", || {
        for path in &src.hgt {
            match HgtTile::read(path) {
                Ok(tile) => dem.tiles.push(tile),
                Err(e) => eprintln!("{}: {e} (no elevation)", path.display()),
            }
        }
    });
    let files = dem_files(&src.dem);
    if files.is_empty() {
        return dem;
    }
    let bounds = load_box(near);
    let threads = ocw_worldgen::par::threads().min(12);
    let cell = src.dem_cell;
    // Files outside the region cost only their header: each thread takes
    // the next file (one sorted chunk per thread left most threads idle).
    stage_start("DTM GeoTIFF read + resample");
    let results: Vec<Result<Option<GridTile>, String>> =
        ocw_worldgen::par::map(&files, threads, |f| read_dem_tile(f, bounds, cell));
    stage_end();
    stage_start("DTM layers");
    let (mut outside, mut bad) = (0, 0);
    for r in results {
        match r {
            Ok(Some(tile)) => {
                if let Err(e) = dem.add_grid(tile) {
                    eprintln!("dem: {e}");
                    bad += 1;
                }
            }
            Ok(None) => outside += 1,
            Err(e) => {
                eprintln!("{e}");
                bad += 1;
            }
        }
    }
    dem.finish();
    stage_end();
    for (i, l) in dem.grids.iter().enumerate() {
        println!(
            "dem layer {i}: {} tiles of {} x {} samples at {} m ({:?}), {:.0} MB",
            l.len(),
            l.nx,
            l.ny,
            l.cell,
            l.crs(),
            (l.len() * l.nx * l.ny * 4) as f64 / 1e6
        );
    }
    println!(
        "dem: {} GeoTIFF files, {outside} outside the region, {bad} unusable; SRTM fallback {} tiles; {:.1} s",
        files.len(),
        dem.tiles.len(),
        t.elapsed().as_secs_f64()
    );
    dem
}

/// The region of a package or route: the `--poly` region, narrowed to
/// `--bbox` (a part of it, spec §2.2.2), with the projection origin
/// (`--origin`, default the centre of the part). For a part also the wider
/// context region (CONTEXT_DEG beyond the box) whose roads shape the
/// profiles and the terrain near its border, so the neighbouring part,
/// built the same way, meets it without a step.
fn cut_region(
    polys: &[PathBuf],
    bbox: Option<Region>,
    origin: Option<(f64, f64)>,
) -> (Region, Option<Region>) {
    let mut region = load_region(polys);
    let mut context = bbox.as_ref().map(|b| {
        region.clone().clipped_to(
            b.min_lat - CONTEXT_DEG,
            b.min_lon - CONTEXT_DEG,
            b.max_lat + CONTEXT_DEG,
            b.max_lon + CONTEXT_DEG,
        )
    });
    if let Some(b) = bbox {
        region = region.clipped_to(b.min_lat, b.min_lon, b.max_lat, b.max_lon);
        if region.min_lat >= region.max_lat || region.min_lon >= region.max_lon {
            eprintln!("--bbox does not overlap the --poly region");
            std::process::exit(1);
        }
    }
    region.origin = origin;
    if let Some(c) = context.as_mut() {
        c.origin = Some(region.origin());
    }
    (region, context)
}

/// What the cleaning passes take from the OSM areas (spec §4.8).
struct AreaInputs {
    data: ocw_worldgen::areas::AreaData,
    areas: Vec<ocw_worldgen::landuse::Area>,
    /// The sea from the coastline (spec §4.6), kept out of `areas`: the
    /// bridge water test would bucket it over the whole region.
    sea: ocw_worldgen::landuse::Sea,
    lines: Vec<ocw_worldgen::landuse::WaterLine>,
    water: ocw_worldgen::water::WaterBodies,
    outlines: Vec<Vec<Vec<[f64; 2]>>>,
}

/// The sea's frame reaches this far beyond the region's box (the margin
/// tiles around its roads, spec §4.6).
const SEA_FRAME_MARGIN_M: f64 = 5000.0;

fn read_area_inputs(
    pbfs: &[PathBuf],
    near: &Region,
    proj: &ocw_world::geo::LocalProjection,
) -> AreaInputs {
    let mut data = stage("read PBF areas", || {
        ocw_worldgen::areas::AreaData::read_pbfs(pbfs).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        })
    });
    stage("retain near", || data.retain_near(load_box(near)));
    let (areas, lines) = stage("land-use areas", || {
        ocw_worldgen::landuse::extract(&data, proj)
    });
    // The sea: the coastline's islands cut out of the region's box plus
    // the margin tiles (spec §4.6).
    let (s, w, n, e) = load_box(near);
    let corners = [(s, w), (s, e), (n, w), (n, e)].map(|(la, lo)| proj.project(la, lo));
    let m = SEA_FRAME_MARGIN_M;
    let frame = [
        corners.iter().map(|c| c[0]).fold(f64::MAX, f64::min) - m,
        corners.iter().map(|c| c[1]).fold(f64::MAX, f64::min) - m,
        corners.iter().map(|c| c[0]).fold(f64::MIN, f64::max) + m,
        corners.iter().map(|c| c[1]).fold(f64::MIN, f64::max) + m,
    ];
    let sea = stage("sea", || ocw_worldgen::landuse::sea(&data, proj, frame));
    if sea.open > 0 {
        eprintln!(
            "warning: {} coastline pieces do not close in the package area: no sea (closing a coast along the frame is not done yet)",
            sea.open
        );
    }
    let water = stage("water bodies", || {
        ocw_worldgen::water::extract_water(&data, proj)
    });
    let outlines = stage("bridge outlines", || {
        ocw_worldgen::bridges::bridge_outlines(&data, proj)
    });
    AreaInputs {
        data,
        areas,
        sea,
        lines,
        water,
        outlines,
    }
}

/// A road graph with its profiles and, when cleaning (spec §4.8), its
/// cleaned centrelines, snapped ends, eased junction mouths, vertical
/// curves and detected bridge spans (with straight decks).
fn prepared_graph(
    osm: &OsmData,
    dem: &Dem,
    region: &Region,
    proj: &ocw_world::geo::LocalProjection,
    clean: Option<&ocw_worldgen::bridges::WaterTest>,
    log: bool,
) -> (
    ocw_world::graph::RoadGraph,
    ocw_worldgen::build::BuildStats,
    Vec<ocw_worldgen::bridges::Span>,
) {
    use ocw_worldgen::clean;
    let (mut graph, stats) = stage("build graph", || build_road_graph(osm, region));
    let mut snaps = Vec::new();
    if clean.is_some() {
        let h = stage("clean polylines", || clean::clean_polylines(&mut graph));
        let (s, hs) = stage("snap near misses", || clean::snap_near_misses(&mut graph));
        snaps = s;
        if log {
            println!(
                "clean: {} duplicate nodes and {} spikes removed, {} road ends snapped (up to {:.1} m)",
                h.duplicates_removed, h.spikes_removed, hs.ends_snapped, hs.max_snap_m
            );
        }
    }
    stage("profiles", || add_profiles(&mut graph, dem, proj));
    let mut spans = Vec::new();
    if let Some(water) = clean {
        let (tied, mouths, curves) = stage("tie ends, junction mouths, vertical curves", || {
            (
                clean::tie_snapped_heights(&mut graph, &snaps),
                clean::ease_junction_mouths(&mut graph),
                clean::limit_vertical_curvature(&mut graph),
            )
        });
        let ground = |p: [f64; 2]| {
            let (lat, lon) = proj.unproject(p);
            dem.elevation(lat, lon)
        };
        spans = stage("bridge detection", || {
            ocw_worldgen::bridges::detect_spans(&graph, water, &ground)
        });
        let planned = stage("bridge decks", || {
            ocw_worldgen::bridges::plan_span_profiles(&mut graph, &spans)
        });
        if log {
            use ocw_world::bridges::BridgeSource;
            let count = |src| spans.iter().filter(|s| s.source == src).count();
            println!(
                "clean: {tied} snapped ends tied in height, {mouths} junction mouths eased, {curves} profile samples rounded; detected bridges: {} over water, {} over gaps, {} in bridge outlines ({planned} deck samples planned)",
                count(BridgeSource::Water),
                count(BridgeSource::Gap),
                count(BridgeSource::Outline)
            );
        }
    }
    (graph, stats, spans)
}

/// A road graph and its cleaning results ([`profiled_graph`]).
struct Profiled {
    graph: ocw_world::graph::RoadGraph,
    stats: ocw_worldgen::build::BuildStats,
    spans: Vec<ocw_worldgen::bridges::Span>,
    /// For a `--bbox` part: the context graph and its spans.
    context: Option<(
        ocw_world::graph::RoadGraph,
        Vec<ocw_worldgen::bridges::Span>,
    )>,
}

impl Profiled {
    /// The graph the terrain is conformed to and its bridge spans.
    fn terrain(&self) -> (&ocw_world::graph::RoadGraph, Vec<(usize, f64, f64)>) {
        let (g, spans) = match &self.context {
            Some((g, s)) => (g, s),
            None => (&self.graph, &self.spans),
        };
        (
            g,
            spans.iter().map(|s| (s.edge, s.from_m, s.to_m)).collect(),
        )
    }
}

/// The road graph of a region with its elevation profiles; for a `--bbox`
/// part the profiles come from the context roads around it (the context
/// graph is returned for the terrain), exactly as the package is built, so
/// a route file cut from the same part has the package's road heights.
/// `clean`: the world cleaning (spec §4.8) with its water test.
fn profiled_graph(
    osm: &OsmData,
    dem: &Dem,
    region: &Region,
    context: Option<Region>,
    clean: Option<&ocw_worldgen::bridges::WaterTest>,
) -> Profiled {
    let proj = region.projection();
    let (mut graph, stats, spans) = stage("region graph", || {
        prepared_graph(osm, dem, region, &proj, clean, true)
    });
    let context = context.map(|c| {
        let (g, _, cspans) = stage("context graph", || {
            prepared_graph(osm, dem, &c, &proj, clean, false)
        });
        let taken = stage("transfer profiles", || {
            ocw_worldgen::profile::transfer_profiles(&mut graph, &g, 0.5)
        });
        println!(
            "context: {} road edges up to {CONTEXT_DEG}° around the box; {taken} of {} profiles taken from them",
            g.edges.len(),
            graph.edges.len()
        );
        (g, cspans)
    });
    Profiled {
        graph,
        stats,
        spans,
        context,
    }
}

fn route(args: RouteArgs) {
    let (region, context) = cut_region(&args.src.poly, args.bbox, args.origin);
    let proj = region.projection();
    let near = context.clone().unwrap_or_else(|| region.clone());
    let osm = read_osm(&args.src.pbf, &near);
    let dem = load_dem(&args.src, &near);
    // The same cleaning as the package, so the route has its road heights.
    let inputs = (!args.no_clean).then(|| read_area_inputs(&args.src.pbf, &near, &proj));
    let water = inputs
        .as_ref()
        .map(|i| ocw_worldgen::bridges::WaterTest::new(&i.areas, &i.lines, &i.outlines));
    let graph = profiled_graph(&osm, &dem, &region, context, water.as_ref()).graph;
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
        origin: region.origin(),
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

/// How far beyond a `--bbox` part the roads shape its profiles and terrain
/// (degrees, about 1 km).
const CONTEXT_DEG: f64 = 0.01;

struct RouteArgs {
    src: SourceArgs,
    from: (f64, f64),
    to: (f64, f64),
    name: String,
    margin: f64,
    out: PathBuf,
    bbox: Option<Region>,
    origin: Option<(f64, f64)>,
    no_clean: bool,
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
    src: SourceArgs,
    name: String,
    samples: u32,
    margin_tiles: i32,
    tiles_near_roads: bool,
    land_tiles: bool,
    landuse_cells: u32,
    no_areas: bool,
    no_clean: bool,
    bbox: Option<Region>,
    origin: Option<(f64, f64)>,
    time_zone: Option<String>,
    out: PathBuf,
}

/// The time zone a package gets (spec §14 Phase 9): `--time-zone` (checked
/// against the tzdb), else the zone of the region at the origin.
fn package_time_zone(given: Option<String>, origin: (f64, f64)) -> Result<String, String> {
    let zone = given.unwrap_or_else(|| ocw_sky::zone::zone_for(origin.0, origin.1));
    ocw_sky::Zone::get(&zone).map(|z| z.id().to_owned())
}

fn package(args: PackageArgs) {
    let PackageArgs {
        src,
        name,
        samples,
        margin_tiles,
        tiles_near_roads,
        land_tiles,
        landuse_cells,
        no_areas,
        no_clean,
        bbox,
        origin,
        time_zone,
        out,
    } = args;
    let t = Instant::now();
    let (region, context) = cut_region(&src.poly, bbox, origin);
    let proj = region.projection();
    let near = context.clone().unwrap_or_else(|| region.clone());
    let osm = stage("read OSM roads", || read_osm(&src.pbf, &near));
    let dem = stage("elevation (DEM)", || load_dem(&src, &near));
    let clean = !no_clean;
    let ta = Instant::now();
    let inputs = (!no_areas || clean).then(|| {
        stage("read OSM areas", || {
            read_area_inputs(&src.pbf, &near, &proj)
        })
    });
    let read_s = ta.elapsed().as_secs_f64();
    let water_test = inputs.as_ref().filter(|_| clean).map(|i| {
        stage("water test index", || {
            ocw_worldgen::bridges::WaterTest::new(&i.areas, &i.lines, &i.outlines)
        })
    });
    let profiled = stage("road graph", || {
        profiled_graph(&osm, &dem, &region, context, water_test.as_ref())
    });
    let (graph, stats) = (&profiled.graph, &profiled.stats);
    let base = |x: f64, z: f64| {
        let (lat, lon) = proj.unproject([x, z]);
        dem.elevation(lat, lon).unwrap_or(0.0)
    };
    let (terrain_graph, terrain_spans) = profiled.terrain();
    // Cleaning: each road cuts through its nearest point only, with a small
    // margin; the terrain is settled under the strips afterwards (§4.8.3).
    let cut = if clean {
        0.5
    } else {
        ocw_worldgen::package::cut_margin_m(1000.0, samples)
    };
    stage_start("terrain conforming + tiles");
    let mut terrain = stage("terrain model", || {
        ocw_worldgen::package::GraphTerrain::new(terrain_graph, base, cut)
            .with_bridge_spans(terrain_spans.iter().copied())
    });
    if clean {
        terrain = terrain.nearest_point_only();
    }
    let mut pkg = ocw_worldgen::package::build_package_on(
        &name,
        region.origin(),
        graph,
        &terrain,
        1000.0,
        samples,
        margin_tiles,
        if land_tiles {
            ocw_worldgen::package::TileCover::NearRoadsAndLand
        } else if tiles_near_roads {
            ocw_worldgen::package::TileCover::NearRoads
        } else {
            ocw_worldgen::package::TileCover::Box
        },
    );
    stage_end();
    pkg.region = Some(region.package_region());
    // Road surfaces from OSM `surface` (section RSRF, Phase 8 T2), on the
    // strips as built (before any cleaning).
    let way_surfaces = ocw_worldgen::filter::way_surfaces(&osm);
    let surfaced = ocw_worldgen::package::apply_road_surfaces(&mut pkg, graph, |e| {
        way_surfaces
            .get(&e.osm_way)
            .copied()
            .unwrap_or_else(|| ocw_world::RoadSurface::of_class(e.class))
    });
    let zone = package_time_zone(time_zone, region.origin()).unwrap_or_else(|e| {
        eprintln!("--time-zone: {e}");
        std::process::exit(2);
    });
    println!("time zone: {zone}");
    pkg.time_zone = Some(zone);
    let terrain_s = t.elapsed().as_secs_f64();
    let mut area_report = vec![format!(
        "road surfaces (RSRF): {surfaced} strips differ from their category's default"
    )];
    let road_zone = stage("road zone index", || {
        ocw_worldgen::clean::RoadZone::new(terrain_graph, &terrain_spans)
    });
    let spacing = pkg.spacing_m();
    let ground = |p: [f64; 2]| {
        let (lat, lon) = proj.unproject(p);
        dem.elevation(lat, lon)
    };
    let footprints = inputs.as_ref().filter(|_| !no_areas).map(|i| {
        stage("building footprints", || {
            ocw_worldgen::buildings::footprints(&i.data, &proj)
        })
    });
    if let (false, Some(i)) = (no_areas, inputs.as_ref()) {
        let tl = Instant::now();
        let ls = stage("land use", || {
            ocw_worldgen::landuse::add_landuse_with_sea(
                &mut pkg,
                &i.areas,
                i.sea.area.as_ref(),
                &i.lines,
                landuse_cells,
            )
        });
        let landuse_s = tl.elapsed().as_secs_f64();
        area_report.push(format!(
            "areas: read {} areas, {} waterways, {} footprints in {read_s:.1} s",
            ls.areas,
            ls.water_lines,
            footprints.as_ref().map_or(0, Vec::len)
        ));
        area_report.push(format!(
            "sea: {} ({} coastline rings in the area, {} open pieces)",
            if i.sea.area.is_some() {
                "yes, level 0 m"
            } else {
                "none"
            },
            i.sea.islands,
            i.sea.open
        ));
        area_report.push(format!(
            "land use (LUSE, {landuse_cells} cells/tile): {}; {} vertices under water, {} with a water level; {landuse_s:.1} s",
            landuse_summary(&ls.cells),
            ls.water_vertices,
            ls.level_vertices
        ));
        if clean {
            let lc = stage("clean land use", || {
                ocw_worldgen::clean::clean_landuse(pkg.landuse.as_mut().expect("land use"))
            });
            // Under a road the terrain keeps its road-conformed height.
            let road = |p: [f64; 2]| {
                road_zone
                    .covers(p, spacing)
                    .then(|| terrain.height(p[0], p[1]))
            };
            let ws = stage("water levels", || {
                ocw_worldgen::water::level_water(&mut pkg, &i.water, &ground, &road)
            });
            area_report.push(format!(
                "clean: land use {} slivers and {} gaps filled ({} cells); water {} river chains and {} lakes levelled ({} vertices, largest change {:.1} m), {} vertices under roads kept at road level",
                lc.slivers,
                lc.gaps,
                lc.cells,
                ws.river_chains,
                ws.lakes,
                ws.relevelled,
                ws.max_change_m,
                ws.restored_under_roads
            ));
        }
    }
    if clean {
        if let (Some(fps), Some(wt)) = (footprints.as_ref(), water_test.as_ref()) {
            let outlines: Vec<Vec<[f64; 2]>> = fps.iter().map(|f| f.outer.clone()).collect();
            let keep = |p: [f64; 2]| road_zone.covers(p, spacing) || wt.is_water(p);
            let st = stage("building sites", || {
                ocw_worldgen::clean::level_building_sites(&mut pkg, &outlines, &keep)
            });
            area_report.push(format!(
                "clean: {} building sites levelled ({} vertices, up to {:.1} m)",
                st.sites, st.vertices, st.max_change_m
            ));
        }
        let ts = Instant::now();
        let ss = stage("settle terrain under roads", || {
            ocw_worldgen::clean::settle_terrain_under_roads(&mut pkg, 0.05)
        });
        area_report.push(format!(
            "clean: terrain settled under roads: {} samples poked through, {} vertices lowered (up to {:.2} m); {:.1} s",
            ss.violations,
            ss.vertices_lowered,
            ss.max_lowered_m,
            ts.elapsed().as_secs_f64()
        ));
    }
    if let Some(fps) = footprints.as_ref() {
        let tb = Instant::now();
        let bs = stage("buildings", || {
            ocw_worldgen::buildings::add_buildings(&mut pkg, fps)
        });
        let buildings_s = tb.elapsed().as_secs_f64();
        area_report.push(format!(
            "buildings (BLDG): {} placed ({} gable roofs, {} with courtyards), {} outside the tiles; {buildings_s:.1} s",
            bs.placed, bs.gabled, bs.with_holes, bs.outside
        ));
    }
    if clean {
        let ways: std::collections::HashMap<i64, &ocw_worldgen::osm::Tags> =
            osm.ways.iter().map(|w| (w.id, &w.tags)).collect();
        let tags = |id: i64| ways.get(&id).map(|t| (*t).clone());
        let spans = ocw_worldgen::bridges::all_spans(graph, &profiled.spans);
        let bs = stage("bridges", || {
            ocw_worldgen::bridges::make_bridges(&mut pkg, &spans, &tags)
        });
        area_report.push(format!(
            "bridges (BRDG): {} tagged, {} in outlines, {} over water and {} over gaps detected; {} beam, {} arch, {} truss, {} suspension, {} covered; {} piers, {} arches, {:.1} km",
            bs.by_source[0],
            bs.by_source[1],
            bs.by_source[2],
            bs.by_source[3],
            bs.by_structure[0],
            bs.by_structure[1],
            bs.by_structure[2],
            bs.by_structure[3],
            bs.by_structure[4],
            bs.piers,
            bs.arches,
            bs.length_m / 1000.0
        ));
    }
    let bytes = stage("serialise", || pkg.to_bytes());
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    stage("write", || {
        std::fs::write(&out, &bytes).expect("write package")
    });
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

/// The stage profile of the build (spec §4.7): the table on stdout and,
/// with `--profile FILE`, the JSON.
fn print_stage_profile(json: Option<&std::path::Path>) {
    let stages = ocw_worldgen::stages::stages();
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!(
        "build profile ({threads} threads):\n{}",
        ocw_worldgen::stages::report(&stages, threads)
    );
    if let Some(path) = json
        && let Err(e) = std::fs::write(path, ocw_worldgen::stages::json(&stages, threads))
    {
        eprintln!("{}: {e}", path.display());
    }
}

/// The OSM `roof:colour` of the footprint containing a point (spec §14
/// Phase 8 T3: it wins over the orthophoto), from the extracts' buildings.
struct OsmRoofColours {
    cell: f64,
    index: std::collections::HashMap<(i64, i64), Vec<usize>>,
    rings: Vec<(Vec<[f64; 2]>, [u8; 3])>,
}

impl OsmRoofColours {
    fn read(pbfs: &[PathBuf], pkg: &ocw_world::WorldPackage) -> Self {
        let mut out = Self {
            cell: 100.0,
            index: Default::default(),
            rings: Vec::new(),
        };
        if pbfs.is_empty() {
            return out;
        }
        let mut data = ocw_worldgen::areas::AreaData::read_pbfs(pbfs).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
        let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
        let size = f64::from(pkg.tile_size_m);
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for k in pkg.tiles.keys() {
            let o = k.origin(size);
            lo = [lo[0].min(o[0]), lo[1].min(o[1])];
            hi = [hi[0].max(o[0] + size), hi[1].max(o[1] + size)];
        }
        let (n, w) = proj.unproject(lo);
        let (s, e) = proj.unproject(hi);
        data.retain_near((s.min(n), w.min(e), s.max(n), w.max(e)));
        for fp in ocw_worldgen::buildings::footprints(&data, &proj) {
            let Some(rgb) = fp
                .tags
                .get("roof:colour")
                .or_else(|| fp.tags.get("roof:color"))
                .and_then(|v| ocw_worldgen::calibrate::parse_colour(v))
            else {
                continue;
            };
            let i = out.rings.len();
            let (mut a, mut b) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in &fp.outer {
                a = [a[0].min(p[0]), a[1].min(p[1])];
                b = [b[0].max(p[0]), b[1].max(p[1])];
            }
            for cx in (a[0] / out.cell).floor() as i64..=(b[0] / out.cell).floor() as i64 {
                for cz in (a[1] / out.cell).floor() as i64..=(b[1] / out.cell).floor() as i64 {
                    out.index.entry((cx, cz)).or_default().push(i);
                }
            }
            out.rings.push((fp.outer, rgb));
        }
        out
    }

    fn at(&self, p: [f64; 2]) -> Option<[u8; 3]> {
        let k = (
            (p[0] / self.cell).floor() as i64,
            (p[1] / self.cell).floor() as i64,
        );
        self.index.get(&k)?.iter().find_map(|&i| {
            let (ring, rgb) = &self.rings[i];
            ocw_worldgen::areas::inside_rings(p, std::slice::from_ref(ring)).then_some(*rgb)
        })
    }
}

/// `world-builder calibrate` (spec §14 Phase 8 T3).
fn calibrate(
    world: &std::path::Path,
    imagery: &std::path::Path,
    pbf: &[PathBuf],
    season_cells: u32,
    out: Option<PathBuf>,
    key: Option<&ocw_world::manifest::SigningKeyPair>,
) {
    let t = Instant::now();
    let mut pkg = stage("read package", || read_package(world));
    let before = std::fs::metadata(world).map_or(0, |m| m.len());
    let img = stage("load imagery", || {
        ocw_worldgen::imagery::Imagery::load(imagery).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        })
    });
    if img.is_empty() {
        eprintln!(
            "{}: no imagery (run scripts/fetch-imagery.sh)",
            imagery.display()
        );
        std::process::exit(1);
    }
    for o in &img.ortho {
        println!(
            "orthophoto: {} ({}, {} m/px, {} cells of {} m)",
            o.meta.name,
            o.meta.licence,
            o.pixel_m,
            o.cells.len(),
            o.cell_m
        );
    }
    if let Some(s) = &img.seasonal {
        let seasons: Vec<String> = s
            .composites
            .iter()
            .map(|c| format!("{} ({} scenes)", c.0.season.name(), c.0.scenes))
            .collect();
        println!(
            "seasonal: {} ({} m/px, {} × {}): {}",
            s.meta.name,
            s.pixel_m,
            s.width,
            s.height,
            seasons.join(", ")
        );
    }
    let roofs = stage("OSM roof colours", || OsmRoofColours::read(pbf, &pkg));
    println!("OSM roof:colour: {} footprints", roofs.rings.len());
    let osm = |p: [f64; 2]| roofs.at(p);
    let stats = stage("calibrate (PALT, ROOF, SEAS)", || {
        ocw_worldgen::calibrate::calibrate(
            &mut pkg,
            &img,
            &osm,
            ocw_worldgen::calibrate::CalibrateOptions {
                season_cells,
                threads: 0,
            },
        )
    });
    for e in stats.errors.iter().take(10) {
        eprintln!("warning: {e}");
    }
    if stats.errors.len() > 10 {
        eprintln!("warning: … {} more imagery errors", stats.errors.len() - 10);
    }
    println!("palette (PALT):");
    for (k, rgb, n) in &stats.palette {
        println!(
            "  {k:?}: #{:02x}{:02x}{:02x} ({n} px)",
            rgb[0], rgb[1], rgb[2]
        );
    }
    println!(
        "roofs (ROOF): {} buildings, {} from the orthophoto, {} from OSM roof:colour, {} without",
        stats.buildings, stats.roofs_ortho, stats.roofs_osm, stats.roofs_none
    );
    if stats.season_tiles > 0 {
        println!(
            "seasons (SEAS): {} tiles × {} maps of {season_cells}², {:.2} MB ({:.1} KB per tile), {} cells filled, {} tiles outside the composites",
            stats.season_tiles,
            pkg.look.seasons.as_ref().map_or(0, |s| s.seasons.len()),
            stats.season_bytes as f64 / 1e6,
            stats.season_bytes as f64 / 1e3 / stats.season_tiles as f64,
            stats.season_filled,
            stats.season_tiles_outside
        );
    }
    let path = out.unwrap_or_else(|| world.to_path_buf());
    let bytes = stage("serialise", || pkg.to_bytes());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    stage("write", || {
        std::fs::write(&path, &bytes).unwrap_or_else(|e| {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        })
    });
    print_sections(&bytes);
    println!(
        "{}: {:.1} MB -> {:.1} MB ({:+.1} MB, {:+.1} %), {:.1} s",
        path.display(),
        before as f64 / 1e6,
        bytes.len() as f64 / 1e6,
        (bytes.len() as f64 - before as f64) / 1e6,
        100.0 * (bytes.len() as f64 - before as f64) / before.max(1) as f64,
        t.elapsed().as_secs_f64()
    );
    // The manifest gets the new hash and the imagery's notices.
    if path != world {
        let _ = std::fs::copy(dist::sidecar(world), dist::sidecar(&path));
    }
    let m = stage("manifest + hash", || {
        dist::write_manifest(
            &path,
            &dist::ManifestArgs {
                id: None,
                name: None,
                version: None,
                dem_attribution: Vec::new(),
            },
            key,
        )
    });
    for a in &m.manifest.attribution {
        println!("  credit: {} ({})", a.source, a.licence);
    }
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

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dz * dz;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dz).powi(2)).sqrt()
}

/// `world-builder areas`: which OSM areas and waterways shape a point.
fn areas_at(pbf: &std::path::Path, (lat, lon): (f64, f64), radius: f64) {
    let data = ocw_worldgen::areas::AreaData::read_pbf(pbf).unwrap_or_else(|e| {
        eprintln!("{}: {e}", pbf.display());
        std::process::exit(1);
    });
    let proj = ocw_world::geo::LocalProjection::new(lat, lon);
    let local = |ring: &Vec<(f64, f64)>| -> Vec<[f64; 2]> {
        ring.iter().map(|&(a, b)| proj.project(a, b)).collect()
    };
    let tags = |t: &ocw_worldgen::osm::Tags| {
        let mut v: Vec<String> = t.iter().map(|(k, v)| format!("{k}={v}")).collect();
        v.sort();
        v.join(" ")
    };
    for a in data.areas() {
        let rings: Vec<Vec<[f64; 2]>> = a.outer.iter().chain(&a.inner).map(local).collect();
        if ocw_worldgen::areas::inside_rings([0.0, 0.0], &rings) {
            let points: usize = rings.iter().map(Vec::len).sum();
            println!(
                "{} {} ({} rings, {points} points): {:?}  {}",
                if a.id < 0 { "relation" } else { "way" },
                a.id.abs(),
                rings.len(),
                ocw_worldgen::landuse::classify(a.tags),
                tags(a.tags)
            );
        }
    }
    for (w, pts) in data.waterways() {
        let p: Vec<[f64; 2]> = local(&pts);
        let d = p
            .windows(2)
            .map(|s| segment_distance([0.0, 0.0], s[0], s[1]))
            .fold(f64::INFINITY, f64::min);
        if d <= radius {
            println!(
                "waterway way {} {d:.1} m away, width {:?} m: {}",
                w.id,
                ocw_worldgen::landuse::waterway_width(&w.tags),
                tags(&w.tags)
            );
        }
    }
}

/// `world-builder dem-import`: XYZ / GeoTIFF tiles → averaged GeoTIFFs.
fn dem_import(
    crs: ocw_worldgen::crs::Crs,
    cell: f64,
    tile_m: f64,
    out: &std::path::Path,
    files: &[PathBuf],
) {
    let t = Instant::now();
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("{}: {e}", out.display());
        std::process::exit(1);
    }
    let import = |f: &PathBuf| -> Result<String, String> {
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("tile");
        let target = out.join(format!("{stem}.tif"));
        if target.exists() {
            return Ok(format!("{}: have it", target.display()));
        }
        let err = |e: ocw_worldgen::dem::DemError| format!("{}: {e}", f.display());
        let ext = f
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let tile = match ext.as_str() {
            "tif" | "tiff" => GridTile::read_geotiff(f, Some(crs)).map_err(err)?,
            _ => GridTile::read_xyz(f, crs).map_err(err)?,
        };
        let factor = (cell / tile.cell).round();
        if factor < 1.0 || (factor * tile.cell - cell).abs() > 1e-6 {
            return Err(format!(
                "{}: --cell {cell} is not a multiple of its {} m",
                f.display(),
                tile.cell
            ));
        }
        // Grids with their cell centres on the lattice (IGN MDT05) are
        // moved onto it first, so their tiles join a layer.
        let r = tile
            .on_lattice()
            .padded_to(tile_m)
            .resampled(factor as usize)
            .map_err(err)?;
        r.write_geotiff(&target).map_err(err)?;
        let (lo, hi) = r
            .data
            .iter()
            .filter(|v| !v.is_nan())
            .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        Ok(format!(
            "{}: {} x {} at {} m, {:.1} % data, {lo:.1}-{hi:.1} m",
            target.display(),
            r.nx,
            r.ny,
            r.cell,
            100.0 * r.valid_fraction()
        ))
    };
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(8);
    let per = files.len().div_ceil(threads).max(1);
    let lines: Vec<Result<String, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = files
            .chunks(per)
            .map(|c| scope.spawn(move || c.iter().map(import).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("import thread"))
            .collect()
    });
    let mut failed = 0;
    for l in lines {
        match l {
            Ok(l) => println!("{l}"),
            Err(e) => {
                failed += 1;
                eprintln!("{e}");
            }
        }
    }
    println!(
        "dem-import: {} files, {failed} failed, {:.1} s",
        files.len(),
        t.elapsed().as_secs_f64()
    );
    if failed > 0 {
        std::process::exit(1);
    }
}

/// Lower-left corners (km) of the `size` m tiles on the lattice shifted
/// by `phase` that overlap the lat/lon box.
fn dem_tiles(
    crs: ocw_worldgen::crs::Crs,
    bbox: &Region,
    size: f64,
    phase: (f64, f64),
) -> Vec<(i64, i64)> {
    let b = crs.bounds_of(bbox.min_lat, bbox.min_lon, bbox.max_lat, bbox.max_lon);
    let first = |lo: f64, ph: f64| ((lo - ph) / size).floor() as i64;
    let last = |hi: f64, ph: f64| ((hi - ph) / size).ceil() as i64 - 1;
    let mut out = Vec::new();
    for i in first(b[0], phase.0)..=last(b[2], phase.0) {
        for j in first(b[1], phase.1)..=last(b[3], phase.1) {
            let e = phase.0 + i as f64 * size;
            let n = phase.1 + j as f64 * size;
            out.push(((e / 1000.0).round() as i64, (n / 1000.0).round() as i64));
        }
    }
    out
}

/// `world-builder dem-diff`: DTM minus SRTM where the DTM covers.
fn dem_diff(src: &SourceArgs, bbox: &Region, step: f64) {
    let dem = load_dem(src, bbox);
    let srtm = Dem::from_hgt(dem.tiles.clone());
    let proj = bbox.projection();
    let a = proj.project(bbox.max_lat, bbox.min_lon);
    let b = proj.project(bbox.min_lat, bbox.max_lon);
    let mut d: Vec<f64> = Vec::new();
    let (mut total, mut covered) = (0usize, 0usize);
    let mut z = a[1];
    while z <= b[1] {
        let mut x = a[0];
        while x <= b[0] {
            let (lat, lon) = proj.unproject([x, z]);
            total += 1;
            if dem.source_at(lat, lon) < dem.grids.len()
                && let (Some(h), Some(s)) = (dem.elevation(lat, lon), srtm.elevation(lat, lon))
            {
                covered += 1;
                d.push(h - s);
            }
            x += step;
        }
        z += step;
    }
    if d.is_empty() {
        println!("dem-diff: the DTM covers none of the {total} points");
        return;
    }
    d.sort_by(f64::total_cmp);
    let n = d.len() as f64;
    let mean = d.iter().sum::<f64>() / n;
    let rms = (d.iter().map(|v| v * v).sum::<f64>() / n).sqrt();
    let q = |p: f64| d[((n - 1.0) * p).round() as usize];
    println!(
        "dem-diff: {covered} of {total} points ({:.1} %) in the DTM; DTM - SRTM: mean {mean:+.2} m, RMS {rms:.2} m, p5 {:+.1} p50 {:+.1} p95 {:+.1} m, min {:+.1} max {:+.1} m",
        100.0 * covered as f64 / total.max(1) as f64,
        q(0.05),
        q(0.5),
        q(0.95),
        d[0],
        d[d.len() - 1]
    );
}

/// `world-builder lint`: the world lint report of a package (spec §4.8.1).
fn lint(world: &std::path::Path, pbf: &[PathBuf]) {
    let t = Instant::now();
    let pkg = read_package(world);
    let water = (!pbf.is_empty()).then(|| {
        let mut data = ocw_worldgen::areas::AreaData::read_pbfs(pbf).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
        if let Some(r) = &pkg.region {
            let [s, w, n, e] = r.bounds();
            let m = LOAD_MARGIN_DEG;
            data.retain_near((s - m, w - m, n + m, e + m));
        }
        let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
        ocw_worldgen::water::extract_water(&data, &proj)
    });
    let report = ocw_worldgen::lint::lint(&pkg, water.as_ref());
    print!("{}", ocw_worldgen::lint::format_report(&report, pkg.origin));
    println!("lint: {:.1} s", t.elapsed().as_secs_f64());
}

fn main() {
    match Cli::parse().command {
        Cmd::Areas { pbf, at, radius } => areas_at(&pbf, at, radius),
        Cmd::Lint { world, pbf } => lint(&world, &pbf),
        Cmd::DemImport {
            crs,
            cell,
            tile,
            out,
            files,
        } => dem_import(crs, cell, tile, &out, &files),
        Cmd::DemTiles {
            crs,
            bbox,
            size,
            phase,
        } => {
            for (e, n) in dem_tiles(crs, &bbox, size, phase) {
                println!("{e}_{n}");
            }
        }
        Cmd::DemDiff { src, bbox, step } => dem_diff(&src, &bbox, step),
        Cmd::ImageryCells {
            world,
            crs,
            cell,
            margin,
        } => {
            let pkg = read_package(&world);
            let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
            let cells = ocw_worldgen::imagery::package_cells(
                pkg.tiles.keys().copied(),
                &proj,
                crs,
                f64::from(pkg.tile_size_m),
                cell,
                margin,
            );
            for (i, j) in cells {
                let km = |v: i64| v as f64 * cell / 1000.0;
                println!("{}_{}", km(i), km(j));
            }
        }
        Cmd::Calibrate {
            world,
            imagery,
            pbf,
            season_cells,
            out,
            sign_key,
        } => {
            let key = sign_key.map(|k| {
                ocw_worldstore::keys::read_signing_key(&k).unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                })
            });
            calibrate(&world, &imagery, &pbf, season_cells, out, key.as_ref());
            print_stage_profile(None);
        }
        Cmd::Check { world, peak } => {
            let bytes = std::fs::read(&world).unwrap_or_else(|e| {
                eprintln!("{}: {e}", world.display());
                std::process::exit(1);
            });
            let pkg = ocw_world::WorldPackage::from_bytes(&bytes).unwrap_or_else(|e| {
                eprintln!("{}: {e}", world.display());
                std::process::exit(1);
            });
            print_sections(&bytes);
            println!("tiles: {} ({} m grid)", pkg.tiles.len(), pkg.spacing_m());
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
            let proj = ocw_world::geo::LocalProjection::new(pkg.origin.0, pkg.origin.1);
            let mut missing = 0;
            for &(lat, lon, want) in &peak {
                let p = proj.project(lat, lon);
                match ocw_worldgen::package::highest_near(&pkg, p, 150.0) {
                    Some((h, q)) if (h - want).abs() <= 50.0 => println!(
                        "peak {lat},{lon} ({want:.0} m): terrain {h:.0} m at x {:.0} z {:.0}",
                        q[0], q[1]
                    ),
                    found => {
                        missing += 1;
                        println!(
                            "peak {lat},{lon} ({want:.0} m) MISSING at x {:.0} z {:.0}: {}",
                            p[0],
                            p[1],
                            found.map_or("no tile".into(), |(h, _)| format!("terrain {h:.0} m"))
                        );
                    }
                }
            }
            if let Some(p) = &pkg.look.palette {
                let sources: Vec<&str> = p.sources.iter().map(|s| s.name.as_str()).collect();
                println!(
                    "look: palette of {} colours from {}; {} roof colours; {}",
                    p.entries.len(),
                    sources.join(", "),
                    pkg.look
                        .roofs
                        .as_ref()
                        .map_or(0, |r| r.values().flatten().flatten().count()),
                    pkg.look
                        .seasons
                        .as_ref()
                        .map_or("no seasonal tints".to_string(), |s| {
                            let names: Vec<&str> =
                                s.seasons.iter().map(|i| i.season.name()).collect();
                            format!(
                                "seasonal tints ({}) in {} tiles",
                                names.join(", "),
                                s.tiles.len()
                            )
                        })
                );
            } else {
                println!("look: none (not calibrated)");
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
            if missing > 0 {
                std::process::exit(1);
            }
        }
        Cmd::Package {
            src,
            name,
            samples,
            margin_tiles,
            tiles_near_roads,
            land_tiles,
            landuse_cells,
            no_areas,
            no_clean,
            bbox,
            origin,
            time_zone,
            out,
            manifest,
            sign_key,
            profile,
        } => {
            // Read the key first: a bad key should not cost a whole build.
            let key = sign_key.map(|k| {
                ocw_worldstore::keys::read_signing_key(&k).unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                })
            });
            package(PackageArgs {
                src,
                name: name.clone(),
                samples,
                margin_tiles,
                tiles_near_roads,
                land_tiles,
                landuse_cells,
                no_areas,
                no_clean,
                bbox,
                origin,
                time_zone,
                out: out.clone(),
            });
            stage("manifest + hash", || {
                dist::write_manifest(&out, &manifest.args(Some(name)), key.as_ref());
            });
            print_stage_profile(profile.as_deref());
        }
        Cmd::Keys { command } => match command {
            KeysCmd::InitRoot { name, dir } => dist::keys_init_root(&name, dir),
            KeysCmd::NewSigning {
                name,
                root,
                days,
                out,
            } => dist::keys_new_signing(root, &name, days, out),
        },
        Cmd::Sign {
            package,
            key,
            manifest,
        } => dist::sign(&package, &key, &manifest.args(None)),
        Cmd::Verify {
            package,
            trust_root,
            no_dev_root,
            allow_unsigned,
        } => dist::verify(&package, trust_root, !no_dev_root, allow_unsigned),
        Cmd::Publish { package, out } => dist::publish(&package, &out),
        Cmd::Diff { old, new } => dist::diff(&old, &new),
        Cmd::Gpx {
            world,
            gpx: file,
            name,
            out,
            route_file,
            dry_run,
        } => gpx(world, file, name, out, route_file, dry_run),
        Cmd::Routes {
            world,
            from,
            remove,
            export_gpx,
        } => routes(world, from, remove, export_gpx),
        Cmd::Route {
            world: Some(world),
            from,
            to,
            name,
            ..
        } => package_route(world, from, to, name),
        Cmd::Route {
            src,
            from,
            to,
            name,
            margin,
            out,
            bbox,
            origin,
            no_clean,
            world: None,
        } => route(RouteArgs {
            src,
            from,
            to,
            name,
            margin,
            out,
            bbox,
            origin,
            no_clean,
        }),
        Cmd::Graph {
            src,
            bbox,
            svg,
            svg_grade,
            probe,
        } => {
            let region = match bbox {
                Some(r) => r,
                None => load_region(&src.poly),
            };
            let t = Instant::now();
            let osm = read_osm(&src.pbf, &region);
            let read_s = t.elapsed().as_secs_f64();
            let (mut graph, stats) = build_road_graph(&osm, &region);
            println!(
                "read {} extract(s) ({} nodes, {} highway ways) in {read_s:.2} s",
                src.pbf.len(),
                osm.nodes.len(),
                osm.ways.len()
            );
            println!("{stats:#?}");
            let dem = load_dem(&src, &region);
            if !dem.tiles.is_empty() || !dem.grids.is_empty() {
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
                    src.pbf[0]
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("region"),
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

    /// Packages get the zone of their region (spec §14 Phase 9).
    #[test]
    fn packages_get_their_regions_time_zone() {
        assert_eq!(
            package_time_zone(None, (47.15, 9.55)).unwrap(),
            "Europe/Vaduz"
        );
        assert_eq!(
            package_time_zone(None, (47.92, 7.96)).unwrap(),
            "Europe/Berlin"
        );
        assert_eq!(
            package_time_zone(Some("Europe/Zurich".into()), (47.15, 9.55)).unwrap(),
            "Europe/Zurich"
        );
        assert_eq!(
            package_time_zone(None, (28.27, -16.55)).unwrap(),
            "Atlantic/Canary",
            "Tenerife"
        );
        assert!(package_time_zone(Some("Europe/Atlantis".into()), (0.0, 0.0)).is_err());
    }

    #[test]
    fn dem_tiles_cover_the_box_on_the_lgl_lattice() {
        let utm = ocw_worldgen::crs::Crs::UtmNorth(32);
        // Around the Feldberg: the LGL tile 425_5302 (2 km, odd east).
        let (lat, lon) = utm.inverse(426_000.0, 5_303_000.0);
        let b = Region::bbox(lat - 0.001, lon - 0.001, lat + 0.001, lon + 0.001);
        assert_eq!(dem_tiles(utm, &b, 2000.0, (1000.0, 0.0)), vec![(425, 5302)]);
        let tiles = dem_tiles(utm, &b, 1000.0, (0.0, 0.0));
        assert!(tiles.contains(&(426, 5303)) && tiles.contains(&(425, 5302)));
        assert_eq!(tiles.len(), 4);
    }

    #[test]
    fn source_args_merge_extracts_and_dems() {
        let cli = Cli::try_parse_from([
            "world-builder",
            "package",
            "--pbf",
            "a.pbf",
            "--pbf",
            "b.pbf",
            "--poly",
            "a.poly",
            "--dem",
            "data/dgm1",
            "--dem-cell",
            "4",
            "--dem-attribution",
            "lgl-dgm1",
            "--dem-attribution",
            "srtm",
        ])
        .unwrap();
        let Cmd::Package { src, manifest, .. } = cli.command else {
            panic!("package");
        };
        assert_eq!(src.pbf.len(), 2);
        assert_eq!(src.poly.len(), 1);
        assert_eq!(src.hgt, vec![PathBuf::from("data/pilot/N47E009.hgt")]);
        assert_eq!(src.dem_cell, Some(4.0));
        assert_eq!(manifest.dem_attribution, vec!["lgl-dgm1", "srtm"]);
    }

    #[test]
    fn dem_tiles_outside_the_box_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.tif");
        GridTile {
            crs: ocw_worldgen::crs::Crs::UtmNorth(32),
            cell: 1.0,
            x0: 425_000.0,
            y0: 5_303_000.0,
            nx: 4,
            ny: 4,
            data: vec![1.0; 16],
        }
        .write_geotiff(&f)
        .unwrap();
        assert!(
            read_dem_tile(&f, (10.0, 10.0, 10.1, 10.1), None)
                .unwrap()
                .is_none()
        );
        let t = read_dem_tile(&f, (47.8, 7.9, 47.9, 8.1), Some(2.0))
            .unwrap()
            .unwrap();
        assert_eq!((t.nx, t.cell), (2, 2.0));
        assert!(read_dem_tile(&f, (47.8, 7.9, 47.9, 8.1), Some(1.5)).is_err());
        assert_eq!(dem_files(&[dir.path().to_path_buf()]), vec![f]);
    }

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
