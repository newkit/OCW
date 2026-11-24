//! Dumps the sides of a package's junction surfaces with their faults as
//! JSON lines (spec 4.8.8 *Rule: the sides of a road are as smooth as the
//! road*): per junction with a `side.*` or `junction.mouth.above_main`
//! finding its arms, outline chains and the first rows of the arms'
//! strips, for looking at the causes.
//!
//! ```text
//! cargo run -p ocw-worldgen --release --example side_dump -- PKG.ocwpkg OUT.jsonl [LIMIT]
//! ```

use std::collections::{BTreeMap, HashMap};
use std::io::Write;

use ocw_world::WorldPackage;
use ocw_world::junction::JunctionSet;
use ocw_world::junction::section::{NO_EDGE, strip_edges_of};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (pkg_path, out) = match &args[..] {
        [p, o, ..] => (p, o),
        _ => {
            eprintln!("usage: side_dump PKG.ocwpkg OUT.jsonl [LIMIT]");
            std::process::exit(2);
        }
    };
    let limit: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
    let mut pkg =
        WorldPackage::from_bytes(&std::fs::read(pkg_path).expect("package")).expect("pkg");
    let g = pkg.graph.clone().expect("road graph");
    // RECOMPUTE=1: the junctions as this checkout computes them (the stored
    // outlines and strip edges left out).
    if std::env::var("RECOMPUTE").is_ok_and(|v| v == "1") {
        pkg.junctions = None;
    }

    let js = match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(&g, j.index_for(&g)),
        None => JunctionSet::new(&g),
    };
    let faults = ocw_worldgen::junctions::side_faults(&pkg, &g);
    let above = ocw_worldgen::junctions::mouths_above_main(&pkg, &g);
    let mut by_v: BTreeMap<usize, Vec<serde_json::Value>> = BTreeMap::new();
    for f in &faults {
        by_v.entry(f.vertex).or_default().push(serde_json::json!({
            "kind": f.kind, "at": f.at, "value": f.value}));
    }
    for m in &above {
        by_v.entry(m.vertex).or_default().push(serde_json::json!({
            "kind": format!("above.{}", m.cause), "at": m.at, "value": m.above_m, "edge": m.edge}));
    }
    // ONLY=prefix: only the junctions with a finding of that kind (e.g.
    // `above`, `side.corner`).
    if let Ok(only) = std::env::var("ONLY") {
        by_v.retain(|_, fs| {
            fs.iter()
                .any(|f| f["kind"].as_str().unwrap_or("").starts_with(&only))
        });
    }
    // NEAR=x,z[,r]: only the junctions within r m (default 30) of the
    // point, with or without a finding.
    if let Ok(near) = std::env::var("NEAR") {
        let v: Vec<f64> = near.split(',').filter_map(|t| t.parse().ok()).collect();
        let r = v.get(2).copied().unwrap_or(30.0);
        by_v.retain(|&k, _| {
            let p = g.vertices[k];
            (p[0] - v[0]).hypot(p[1] - v[1]) < r
        });
        for (k, p) in g.vertices.iter().enumerate() {
            if (p[0] - v[0]).hypot(p[1] - v[1]) < r {
                by_v.entry(k).or_default();
            }
        }
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in by_v.values().flatten() {
        *counts
            .entry(f["kind"].as_str().unwrap_or("").to_string())
            .or_default() += 1;
    }
    eprintln!("{counts:?}");
    let (mut patches, mut reasons) = (0usize, BTreeMap::<&str, usize>::new());
    for v in 0..g.vertices.len() {
        if js.own_patch(&g, v).is_some() {
            patches += 1;
        } else if let Some(r) = js.fallback_reason(&g, v) {
            *reasons.entry(r).or_default() += 1;
        }
    }
    eprintln!("patches {patches}, without {reasons:?}");
    let side = js.side_paths();
    let paths = g
        .edges
        .iter()
        .filter(|e| ocw_world::junction::alongside::is_path(e.class))
        .count();
    eprintln!(
        "paths {paths}, along a road (drawn as its side) {}",
        side.iter().filter(|&&x| x).count()
    );
    // How far the paths along a road (ridden on their own profile) lie
    // from the band drawn for them (the nearest road's height at the
    // path's centre line, its edge's kerb of 8 cm and the sidewalk's fall
    // beyond): per profile sample, the difference (m).
    let mut diffs: Vec<f64> = Vec::new();
    for (ei, e) in g.edges.iter().enumerate() {
        if !side[ei] {
            continue;
        }
        let len = e.length_m();
        let cum = {
            let mut c = vec![0.0];
            for w in e.points.windows(2) {
                let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
                c.push(c[c.len() - 1] + l);
            }
            c
        };
        for (i, &h) in e.profile.iter().enumerate() {
            let s = (i as f64 * ocw_world::graph::PROFILE_STEP_M).min(len);
            if s < 5.0 || s > len - 5.0 {
                continue;
            }
            let (p, _) = ocw_world::junction::frame_at(&e.points, &cum, s);
            // The nearest other road.
            let mut best: Option<(f64, f64, f64)> = None;
            for (ri, r) in g.edges.iter().enumerate() {
                if side[ri] || ocw_world::junction::alongside::is_path(r.class) {
                    continue;
                }
                let mut acc = 0.0;
                for w in r.points.windows(2) {
                    let ab = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
                    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
                    let l = l2.sqrt();
                    if l2 > 1e-9 {
                        let t = (((p[0] - w[0][0]) * ab[0] + (p[1] - w[0][1]) * ab[1]) / l2)
                            .clamp(0.0, 1.0);
                        let q = [w[0][0] + ab[0] * t, w[0][1] + ab[1] * t];
                        let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                        if best.is_none_or(|b| d < b.0) {
                            let rh = r.height_at(acc + l * t).unwrap_or(h);
                            best = Some((d, rh, r.width_m / 2.0));
                        }
                    }
                    acc += l;
                }
            }
            if let Some((d, rh, half)) = best {
                let band = rh
                    + 0.08
                    + (d - half).max(0.0) * f64::from(ocw_world::road_dress::SIDEWALK_FALL);
                diffs.push(h - band);
            }
        }
    }
    diffs.sort_by(f64::total_cmp);
    if !diffs.is_empty() {
        let q = |f: f64| diffs[((diffs.len() - 1) as f64 * f) as usize];
        eprintln!(
            "side paths: profile − band at {} samples: min {:.2}, p10 {:.2}, median {:.2}, p90 {:.2}, max {:.2} m",
            diffs.len(),
            q(0.0),
            q(0.1),
            q(0.5),
            q(0.9),
            q(1.0)
        );
    }
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
    let mut w = std::io::BufWriter::new(std::fs::File::create(out).expect("out"));
    for (n, (v, fs)) in by_v.iter().enumerate() {
        if n >= limit {
            break;
        }
        let Some(p) = js.own_patch(&g, *v) else {
            continue;
        };
        let arms: Vec<serde_json::Value> = p
            .arms
            .iter()
            .enumerate()
            .map(|(ai, a)| {
                let e = &g.edges[a.edge];
                let mut strips = Vec::new();
                for &(key, si) in by_edge.get(&(a.edge as u32)).into_iter().flatten() {
                    let s = &pkg.tiles[&key].roads[si];
                    let half = s.width_m / 2.0;
                    let sh = js.shape_strip(&g, a.edge, &s.points, half);
                    let rows: Vec<[[f32; 3]; 2]> = match &sh {
                        Some(sh) => sh.rows.clone(),
                        None => Vec::new(),
                    };
                    // The rows within 15 m of the mouth.
                    let near: Vec<(usize, [[f32; 3]; 2])> = rows
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| {
                            let c = [
                                f64::from(r[0][0] + r[1][0]) / 2.0,
                                f64::from(r[0][2] + r[1][2]) / 2.0,
                            ];
                            (c[0] - a.centre[0]).hypot(c[1] - a.centre[1]) < 15.0
                        })
                        .map(|(i, r)| (i, *r))
                        .collect();
                    strips.push(serde_json::json!({
                        "shaped": sh.is_some(), "n": rows.len(), "rows": near,
                        "dress": format!("{:?}", s.dress.sidewalk_m)}));
                }
                serde_json::json!({
                    "i": ai, "edge": a.edge, "way": e.osm_way, "class": format!("{:?}", a.class),
                    "half": a.half, "len": a.len, "at_from": a.at_from, "dir": a.dir,
                    "origin": a.origin, "mouth_m": a.mouth_m, "centre": a.centre, "t": a.t,
                    "row": a.row, "h": a.h, "grade": a.grade, "blend_m": a.blend_m,
                    "edge_m": a.edge_m, "edge_h": a.edge_h, "cross": a.cross,
                    "name": e.name, "strips": strips,
                    "edge_points": e.points.iter().take(12).collect::<Vec<_>>(),
                })
            })
            .collect();
        let chains: Vec<serde_json::Value> = p
            .chains
            .iter()
            .map(|c| {
                let (j5, ok, lines) = ocw_world::junction::debug_chain(&g, p, c.from_arm);
                serde_json::json!({"from": c.from_arm, "to": c.to_arm, "points": c.points, "j5": j5, "j5_ok": ok, "lines": lines})
            })
            .collect();
        let rec = serde_json::json!({
            "vertex": v, "centre": p.centre, "star": p.is_star(), "members": p.members, "through": p.through,
            "faults": fs, "arms": arms, "chains": chains,
        });
        writeln!(w, "{rec}").expect("write");
    }
}
