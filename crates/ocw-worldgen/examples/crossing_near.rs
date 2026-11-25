//! The painted crossings of a package's road strips and the junction
//! surfaces that swallow them (spec 4.8.8, J9 working tool: a crossing
//! inside a junction surface is drawn at its mouth instead of where OSM has
//! it). Without a point: the count of crossings inside a mouth (and the
//! list with `LIST=1`); with `X,Z [R]`: the crossings near that point.
//!
//! ```text
//! cargo run -p ocw-worldgen --release --example crossing_near -- PKG.ocwpkg [X,Z [R]]
//! ```

use ocw_world::WorldPackage;
use ocw_world::junction::section::{NO_EDGE, strip_edges_of};
use ocw_world::junction::{EndRule, JunctionSet};
use ocw_world::road_dress::MarkingStyle;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pkg = WorldPackage::from_bytes(&std::fs::read(&args[0]).expect("package")).expect("pkg");
    let near: Option<(Vec<f64>, f64)> = args.get(1).map(|a| {
        (
            a.split(',').map(|t| t.parse().expect("x,z")).collect(),
            args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30.0),
        )
    });
    let list = std::env::var("LIST").is_ok_and(|v| v == "1");
    let g = pkg.graph.clone().expect("graph");
    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(&g, j.index_for(&g)),
        None => JunctionSet::new(&g),
    };
    let style = MarkingStyle::default();
    let owned;
    let edges = match pkg.junctions.as_ref().and_then(|j| j.strip_edges_for(&g)) {
        Some(s) => s,
        None => {
            owned = strip_edges_of(&pkg, &g);
            &owned
        }
    };
    let (mut total, mut inside) = (0usize, 0usize);
    let mut keys: Vec<_> = pkg.tiles.keys().copied().collect();
    keys.sort();
    for key in keys {
        let tile = &pkg.tiles[&key];
        let ix = edges.tile(key, tile.roads.len());
        for (i, s) in tile.roads.iter().enumerate() {
            for c in &s.dress.crossings {
                let mut walked = 0.0f32;
                let mut pos = None;
                for w in s.points.windows(2) {
                    let l = ((w[1][0] - w[0][0]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt();
                    if walked + l >= c.along_m {
                        let t = ((c.along_m - walked) / l.max(1e-6)).clamp(0.0, 1.0);
                        pos = Some([
                            f64::from(w[0][0] + (w[1][0] - w[0][0]) * t),
                            f64::from(w[0][2] + (w[1][2] - w[0][2]) * t),
                        ]);
                        break;
                    }
                    walked += l;
                }
                let Some(p) = pos else { continue };
                let Some(ei) = ix.map(|v| v[i]).filter(|&e| e != NO_EDGE) else {
                    continue;
                };
                let ei = ei as usize;
                let e = &g.edges[ei];
                // The crossing's distance from the edge's `from` vertex.
                let mut best = (f64::INFINITY, 0.0);
                let mut acc = 0.0;
                for w in e.points.windows(2) {
                    let ab = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
                    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
                    let l = l2.sqrt();
                    if l2 > 1e-12 {
                        let t = (((p[0] - w[0][0]) * ab[0] + (p[1] - w[0][1]) * ab[1]) / l2)
                            .clamp(0.0, 1.0);
                        let q = [w[0][0] + ab[0] * t, w[0][1] + ab[1] * t];
                        let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                        if d < best.0 {
                            best = (d, acc + l * t);
                        }
                    }
                    acc += l;
                }
                let s_from = best.1;
                let half = f64::from(style.crossing_length_m(c.kind)) / 2.0;
                let len = e.length_m();
                let mut swallowed = None;
                for (k, end) in js.ends(&g, ei).iter().enumerate() {
                    let d = if k == 0 { s_from } else { len - s_from };
                    match end {
                        Some(EndRule::Patch(pt, a)) if d - half < pt.arms[*a].mouth_m => {
                            swallowed = Some((pt.vertex, pt.arms[*a].mouth_m, d));
                        }
                        Some(EndRule::Inside(pt)) => swallowed = Some((pt.vertex, len, d)),
                        _ => {}
                    }
                }
                total += 1;
                if swallowed.is_some() {
                    inside += 1;
                }
                let show = match &near {
                    Some((at, r)) => (p[0] - at[0]).hypot(p[1] - at[1]) < *r,
                    None => list && swallowed.is_some(),
                };
                if show {
                    println!(
                        "edge {ei} {:?} at ({:.1}, {:.1}), {s_from:.1} m from its from vertex; inside a junction surface: {swallowed:?}",
                        c.kind, p[0], p[1]
                    );
                }
            }
        }
    }
    println!("crossings {total}, inside a junction surface (drawn at its mouth) {inside}");
}
