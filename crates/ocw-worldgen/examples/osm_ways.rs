//! Looks up OSM ways by name in an extract and prints their tags and where
//! they lie in a package's local metres (for scene positions and checking
//! the road dress tags, spec §14 Phase 8 T5); with `--stats`, how often the
//! tags of the road dress (sidewalks, kerbs, lanes, cycle lanes, crossings)
//! occur among the rideable roads.
//!
//! ```text
//! cargo run -p ocw-worldgen --release --example osm_ways -- PBF PKG NAME
//! cargo run -p ocw-worldgen --release --example osm_ways -- PBF PKG --stats
//! ```

use std::collections::BTreeMap;

use ocw_world::geo::LocalProjection;
use ocw_worldgen::osm::OsmData;

/// The projection centre from a package's header.
fn origin(pkg: &str) -> (f64, f64) {
    let b = std::fs::read(pkg).expect("package");
    let n = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
    let at = 12 + n;
    let f = |i: usize| f64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    (f(at), f(at + 8))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [pbf, pkg, what] = &args[..] else {
        eprintln!("usage: osm_ways PBF PKG NAME|--stats");
        std::process::exit(2);
    };
    let osm = OsmData::read_pbf(std::path::Path::new(pbf)).expect("PBF");
    let (lat0, lon0) = origin(pkg);
    let proj = LocalProjection::new(lat0, lon0);
    if what == "--stats" {
        let mut stats: BTreeMap<String, usize> = BTreeMap::new();
        let mut roads = 0;
        for w in &osm.ways {
            let Some(_) = ocw_worldgen::filter::rideable(w) else {
                if w.tag("footway") == Some("sidewalk") {
                    *stats.entry("separate footway=sidewalk".into()).or_default() += 1;
                }
                continue;
            };
            roads += 1;
            for (k, v) in &w.tags {
                if k.starts_with("sidewalk")
                    || k.starts_with("cycleway")
                    || k.starts_with("lanes")
                    || k.starts_with("overtaking")
                    || k == "kerb"
                {
                    *stats.entry(format!("{k}={v}")).or_default() += 1;
                }
            }
        }
        for t in osm.node_tags.values() {
            for k in ["highway", "crossing", "crossing:markings", "crossing_ref"] {
                if let Some(v) = t.get(k) {
                    *stats.entry(format!("node {k}={v}")).or_default() += 1;
                }
            }
        }
        println!("rideable ways: {roads}");
        for (k, n) in stats.iter().filter(|(_, n)| **n >= 3) {
            println!("{n:7}  {k}");
        }
        return;
    }
    for w in osm
        .ways
        .iter()
        .filter(|w| w.tag("name").is_some_and(|n| n.contains(what.as_str())))
    {
        let pts: Vec<[f64; 2]> = w
            .nodes
            .iter()
            .filter_map(|n| osm.nodes.get(n))
            .map(|&(lat, lon)| proj.project(lat, lon))
            .collect();
        let mid = pts.get(pts.len() / 2).copied().unwrap_or([0.0, 0.0]);
        let mut tags: Vec<_> = w.tags.iter().map(|(k, v)| format!("{k}={v}")).collect();
        tags.sort();
        println!(
            "way {} at {:.0},{:.0} ({} nodes): {}",
            w.id,
            mid[0],
            mid[1],
            w.nodes.len(),
            tags.join(" ")
        );
        for n in &w.nodes {
            if let (Some(t), Some(&(lat, lon))) = (osm.node_tags.get(n), osm.nodes.get(n)) {
                let p = proj.project(lat, lon);
                let mut t: Vec<_> = t.iter().map(|(k, v)| format!("{k}={v}")).collect();
                t.sort();
                println!("    node {n} at {:.0},{:.0}: {}", p[0], p[1], t.join(" "));
            }
        }
    }
}
