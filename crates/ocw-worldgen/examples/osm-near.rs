//! Diagnostics: the tagged OSM nodes and ways near a point of a package
//! (what an object in the world was built from):
//!
//! ```text
//! cargo run --release -p ocw-worldgen --example osm-near -- PBF LAT,LON X,Z RADIUS [KEY…]
//! ```
//!
//! LAT,LON is the package origin, X,Z the point in package metres; KEYs
//! filter the tags (default: power, aerialway, man_made, natural, tower:type).

use std::collections::HashSet;
use std::path::Path;

use ocw_worldgen::osm::par_elements;
use ocw_worldgen::projection::LocalProjection;

type Found = (i64, [f64; 2], f64, Vec<String>);

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 4 {
        eprintln!("usage: osm-near PBF LAT,LON X,Z RADIUS [KEY…]");
        std::process::exit(2);
    }
    let pair = |s: &str| -> [f64; 2] {
        let v: Vec<f64> = s.split(',').filter_map(|x| x.parse().ok()).collect();
        [v[0], v[1]]
    };
    let o = pair(&a[1]);
    let p = pair(&a[2]);
    let r: f64 = a[3].parse().unwrap_or(300.0);
    let keys: Vec<String> = if a.len() > 4 {
        a[4..].to_vec()
    } else {
        ["power", "aerialway", "man_made", "natural", "tower:type"]
            .map(String::from)
            .to_vec()
    };
    let wanted = |tags: &[String]| {
        tags.iter()
            .any(|t| keys.iter().any(|k| t.starts_with(&format!("{k}="))))
    };
    let proj = LocalProjection::new(o[0], o[1]);
    let near = |lat: f64, lon: f64| {
        let q = proj.project(lat, lon);
        let d = (q[0] - p[0]).hypot(q[1] - p[1]);
        (d <= r).then_some((q, d))
    };
    let path = Path::new(&a[0]);
    let nodes: Vec<Found> = par_elements(path, |el, out: &mut Vec<Found>| {
        let (id, lat, lon, tags): (i64, f64, f64, Vec<(&str, &str)>) = match el {
            osmpbf::Element::Node(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
            osmpbf::Element::DenseNode(n) => (n.id(), n.lat(), n.lon(), n.tags().collect()),
            _ => return,
        };
        if let Some((q, d)) = near(lat, lon) {
            let tags: Vec<String> = tags.iter().map(|(k, v)| format!("{k}={v}")).collect();
            out.push((id, q, d, tags));
        }
    })
    .expect("pbf");
    let ids: HashSet<i64> = nodes.iter().map(|n| n.0).collect();
    for (id, q, d, tags) in &nodes {
        if wanted(tags) {
            println!(
                "node {id} at {:.0},{:.0} ({d:.0} m): {}",
                q[0],
                q[1],
                tags.join(" ")
            );
        }
    }
    let ways: Vec<(i64, Vec<String>)> =
        par_elements(path, |el, out: &mut Vec<(i64, Vec<String>)>| {
            if let osmpbf::Element::Way(w) = el
                && w.refs().any(|n| ids.contains(&n))
            {
                let mut tags: Vec<String> = w.tags().map(|(k, v)| format!("{k}={v}")).collect();
                if wanted(&tags) {
                    let here: Vec<String> = w
                        .refs()
                        .filter(|n| ids.contains(n))
                        .map(|n| n.to_string())
                        .collect();
                    let all: Vec<i64> = w.refs().collect();
                    tags.push(format!(
                        "[nodes near: {}; {} nodes, first {} last {}]",
                        here.join(","),
                        all.len(),
                        all[0],
                        all[all.len() - 1]
                    ));
                    out.push((w.id(), tags));
                }
            }
        })
        .expect("pbf");
    for (id, tags) in ways {
        println!("way {id}: {}", tags.join(" "));
    }
}
