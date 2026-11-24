//! Looks at each `junction.mouth.above_main` finding of a package (spec
//! 4.8.8 Q7, Rule): at its point the side road's strip rows round it (as
//! shaped, with their heights) against the main road's surface the strip
//! cut uses (`Patch::main_road_height`, `main_own`, `main_excess`) – why
//! the cut did not put the strip under the main road there.
//!
//! ```text
//! cargo run -p ocw-worldgen --release --example above_probe -- PKG.ocwpkg
//! ```

use std::collections::HashMap;

use ocw_world::WorldPackage;
use ocw_world::junction::section::{NO_EDGE, strip_edges_of};
use ocw_world::junction::{JunctionSet, MAIN_REACH_M, SIDE_MARGIN_M};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: above_probe PKG.ocwpkg");
    let pkg = WorldPackage::from_bytes(&std::fs::read(&path).expect("package")).expect("pkg");
    let g = pkg.graph.clone().expect("road graph");
    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(&g, j.index_for(&g)),
        None => JunctionSet::new(&g),
    };
    let owned;
    let edges = match pkg.junctions.as_ref().and_then(|j| j.strip_edges_for(&g)) {
        Some(s) => s,
        None => {
            owned = strip_edges_of(&pkg, &g);
            &owned
        }
    };
    let mut by_edge: HashMap<u32, Vec<(ocw_world::TileKey, usize)>> = HashMap::new();
    for (key, tile) in &pkg.tiles {
        if let Some(ix) = edges.tile(*key, tile.roads.len()) {
            for (i, &e) in ix.iter().enumerate() {
                if e != NO_EDGE {
                    by_edge.entry(e).or_default().push((*key, i));
                }
            }
        }
    }
    // GRAPH=vertex: the edges at that junction (its cluster's members) as
    // test data, one per line: `from to class width x,z;x,z;… h,h,…`,
    // coordinates from the patch's centre, vertices numbered from 0.
    if let Some(v) = std::env::var("GRAPH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    {
        let p = js.patch(&g, v).expect("a patch");
        let c = [p.centre[0], p.centre[2]];
        let mut ids: Vec<u32> = Vec::new();
        let mut id = |w: u32| -> usize {
            ids.iter().position(|&x| x == w).unwrap_or_else(|| {
                ids.push(w);
                ids.len() - 1
            })
        };
        let mut edges: Vec<usize> = p
            .arms
            .iter()
            .map(|a| a.edge)
            .chain(p.inner.iter().copied())
            .collect();
        edges.sort_unstable();
        edges.dedup();
        for ei in edges {
            let e = &g.edges[ei];
            let (f, t) = (id(e.from.0), id(e.to.0));
            let pts: Vec<String> = e
                .points
                .iter()
                .map(|q| format!("{:.2},{:.2}", q[0] - c[0], q[1] - c[1]))
                .collect();
            let prof: Vec<String> = e
                .profile
                .iter()
                .map(|h| format!("{:.3}", h - p.centre[1]))
                .collect();
            println!(
                "{f} {t} {:?} {:.2} {} {}",
                e.class,
                e.width_m,
                pts.join(";"),
                prof.join(",")
            );
        }
        return;
    }
    for m in ocw_worldgen::junctions::mouths_above_main(&pkg, &g) {
        let Some(p) = js.own_patch(&g, m.vertex) else {
            continue;
        };
        let q = m.at;
        println!(
            "vertex {} at {:?}: {:.3} m above ({}), edge {} – main_road_height {:?}, main_own {:?}, main_excess {:.2}",
            m.vertex,
            q,
            m.above_m,
            m.cause,
            m.edge,
            p.main_road_height(&g, q, MAIN_REACH_M, SIDE_MARGIN_M),
            p.main_own(q),
            p.main_excess(q)
        );
        if m.edge == usize::MAX {
            continue;
        }
        let ends = js.ends(&g, m.edge);
        println!(
            "  side edge ends: {:?}",
            ends.map(|e| e.map(|r| match r {
                ocw_world::junction::EndRule::Patch(pp, i) =>
                    format!("patch {} arm {i} through {:?}", pp.vertex, pp.through),
                ocw_world::junction::EndRule::Mitre(..) => "mitre".into(),
                ocw_world::junction::EndRule::Inside(pp) => format!("inside {}", pp.vertex),
            }))
        );
        for &(key, si) in by_edge.get(&(m.edge as u32)).into_iter().flatten() {
            let s = &pkg.tiles[&key].roads[si];
            let sh = js.shape_strip(&g, m.edge, &s.points, s.width_m / 2.0);
            let Some(sh) = sh else {
                let near = s
                    .points
                    .iter()
                    .any(|r| (f64::from(r[0]) - q[0]).hypot(f64::from(r[2]) - q[1]) < 6.0);
                if near {
                    println!("  strip in {key:?} unshaped near the point");
                }
                continue;
            };
            for (i, r) in sh.rows.iter().enumerate() {
                let c = [
                    f64::from(r[0][0] + r[1][0]) / 2.0,
                    f64::from(r[0][2] + r[1][2]) / 2.0,
                ];
                if (c[0] - q[0]).hypot(c[1] - q[1]) > 3.0 {
                    continue;
                }
                let info: Vec<String> = r
                    .iter()
                    .map(|k| {
                        let x = [f64::from(k[0]), f64::from(k[2])];
                        format!(
                            "({:.2},{:.2}) y {:.3} main {:?} own {:?} excess {:.2}",
                            x[0],
                            x[1],
                            k[1],
                            p.main_road_height(&g, x, MAIN_REACH_M, SIDE_MARGIN_M)
                                .map(|h| (h * 1000.0).round() / 1000.0),
                            p.main_own(x).map(|h| (h * 1000.0).round() / 1000.0),
                            p.main_excess(x)
                        )
                    })
                    .collect();
                println!("  row {i}: {}", info.join(" | "));
            }
        }
    }
}
