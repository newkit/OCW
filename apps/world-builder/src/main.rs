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
    },
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

fn main() {
    match Cli::parse().command {
        Cmd::Graph {
            pbf,
            poly,
            bbox,
            hgt,
            svg,
            svg_grade,
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
    }
}
