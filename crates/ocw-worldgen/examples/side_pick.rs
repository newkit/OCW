//! Picks junction corners to compare before / after the Rule on the sides
//! (spec 4.8.8 *Rule*, J5): from a package (the J4 one) the corners where
//! the lint found a fault of the sides, one per kind of junction – a T
//! (a through road and a side road), a Y / fork (no through road, two arms
//! less than 50° apart) and a path off a main road (a through road with a
//! path or cycleway leaving it) – and prints for each the camera of
//! `scripts/junction-shots.sh`: 14 m out from the junction over the
//! corner, 7 m up, looking back at it.
//!
//! ```text
//! cargo run -p ocw-worldgen --release --example side_pick -- PKG.ocwpkg [SKIP]
//! ```

use ocw_world::WorldPackage;
use ocw_world::graph::RoadClass;
use ocw_world::junction::JunctionSet;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().expect("usage: side_pick PKG.ocwpkg [SKIP]");
    let skip: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let pkg = WorldPackage::from_bytes(&std::fs::read(path).expect("package")).expect("pkg");
    let g = pkg.graph.clone().expect("road graph");
    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(&g, j.index_for(&g)),
        None => JunctionSet::new(&g),
    };
    let mut faults = ocw_worldgen::junctions::side_faults(&pkg, &g);
    faults.retain(|f| f.kind == "corner" || f.kind == "kink" || f.kind == "step");
    faults.sort_by(|a, b| b.value.total_cmp(&a.value));
    let mut found: Vec<(&str, usize)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in &faults {
        if !seen.insert(f.vertex) {
            continue;
        }
        let Some(p) = js.own_patch(&g, f.vertex) else {
            continue;
        };
        if p.members.len() > 1 || p.arms.len() != 3 {
            continue;
        }
        let path_like = |c: RoadClass| matches!(c, RoadClass::Path | RoadClass::Cycleway);
        let kind = match p.through {
            Some(t) => {
                let side = (0..3).find(|i| !t.contains(i)).expect("side arm");
                if path_like(p.arms[side].class) && !path_like(p.arms[t[0]].class) {
                    "path"
                } else if !path_like(p.arms[side].class) {
                    "t"
                } else {
                    continue;
                }
            }
            None => {
                let close = (0..3).any(|i| {
                    let (a, b) = (p.arms[i].dir, p.arms[(i + 1) % 3].dir);
                    (a[0] * b[0] + a[1] * b[1]).acos().to_degrees() < 50.0
                });
                if close { "fork" } else { continue }
            }
        };
        let n = found.iter().filter(|(k, _)| *k == kind).count();
        if n > skip {
            continue;
        }
        found.push((kind, f.vertex));
        if n < skip {
            continue;
        }
        // The camera over the corner of the fault, looking back.
        let c = [p.centre[0], p.centre[2]];
        let mut d = [f.at[0] - c[0], f.at[1] - c[1]];
        let l = d[0].hypot(d[1]).max(1e-6);
        d = [d[0] / l, d[1] / l];
        let cam = [c[0] + 14.0 * d[0], p.centre[1] + 7.0, c[1] + 14.0 * d[1]];
        // Compass heading of the view (0 north = −z, 90 east = +x).
        let heading = (-d[0]).atan2(d[1]).to_degrees().rem_euclid(360.0);
        let names: Vec<String> = p
            .arms
            .iter()
            .map(|a| {
                g.edges[a.edge]
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", a.class))
            })
            .collect();
        println!(
            "{kind} vertex {} {:.1} {:.2} {:.1} {:.1} -24 {:.0},{:.0} # {} {:.2} at {:.1},{:.1}; {}",
            f.vertex,
            cam[0],
            cam[1],
            cam[2],
            heading,
            c[0],
            c[1],
            f.kind,
            f.value,
            f.at[0],
            f.at[1],
            names.join(" / ")
        );
    }
}
