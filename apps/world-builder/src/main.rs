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
use ocw_worldgen::osm::OsmData;
use ocw_worldgen::svg::graph_svg;

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
        /// Write an SVG map of the graph.
        #[arg(long)]
        svg: Option<PathBuf>,
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
            svg,
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
            let (graph, stats) = build_road_graph(&osm, &region);
            println!(
                "read {} ({} nodes, {} highway ways) in {read_s:.2} s",
                pbf.display(),
                osm.nodes.len(),
                osm.ways.len()
            );
            println!("{stats:#?}");
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
                std::fs::write(&path, graph_svg(&graph, 1400.0, &title)).expect("write svg");
                println!("map: {}", path.display());
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
