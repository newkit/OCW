//! Dumps the side roads of a package's junction surfaces as JSON lines, for
//! comparing two builds of the same region (spec 4.8.8 Q6 (d) / Q7: the
//! creases where a side road leaves the main road's edge at its own grade,
//! the kinks inside blended junctions). Per junction arm: where it is, its
//! names, the rule (`crease` – a side arm with a sloped corner, `side` – a
//! side arm blended as in J2, `main` – an arm of the through road,
//! `blended` – an arm of a junction without a through road) and, for each
//! package, the height a rider meets along its centre line every metre from
//! the vertex (the drawn junction surface, else the road's profile), the raw
//! profile and the intervals steeper than the class limit with their cause.
//!
//! ```text
//! cargo run -p ocw-worldgen --release --example junction_profiles -- LIVE.ocwpkg J3.ocwpkg OUT.jsonl
//! ```

use std::collections::HashMap;
use std::io::Write;

use ocw_world::WorldPackage;
use ocw_world::graph::RoadGraph;
use ocw_world::junction::{EndRule, JunctionSet};

const REACH_M: f64 = 60.0;

fn load(path: &str) -> WorldPackage {
    WorldPackage::from_bytes(&std::fs::read(path).expect("package")).expect("package")
}

fn junctions(pkg: &WorldPackage, g: &RoadGraph) -> JunctionSet {
    match pkg.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(g, j.index_for(g)),
        None => JunctionSet::new(g),
    }
}

/// The point `s` metres along a polyline and its direction.
fn at(points: &[[f64; 2]], s: f64) -> ([f64; 2], [f64; 2]) {
    let mut left = s.max(0.0);
    for w in points.windows(2) {
        let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
        let l = d[0].hypot(d[1]);
        if l < 1e-9 {
            continue;
        }
        let t = [d[0] / l, d[1] / l];
        if left <= l {
            return ([w[0][0] + t[0] * left, w[0][1] + t[1] * left], t);
        }
        left -= l;
    }
    let n = points.len();
    let d = [
        points[n - 1][0] - points[n - 2][0],
        points[n - 1][1] - points[n - 2][1],
    ];
    let l = d[0].hypot(d[1]).max(1e-9);
    (points[n - 1], [d[0] / l, d[1] / l])
}

struct Side {
    js: JunctionSet,
    steep: HashMap<usize, Vec<(usize, f64, &'static str)>>,
}

fn side(pkg: &WorldPackage, g: &RoadGraph) -> Side {
    let js = junctions(pkg, g);
    let decks = ocw_worldgen::junctions::bridge_spans(pkg, g);
    let keep = |e: usize, s: f64| {
        !decks
            .iter()
            .any(|d| d.0 == e && s >= d.1 - 1.0 && s <= d.2 + 1.0)
    };
    let mut steep: HashMap<usize, Vec<(usize, f64, &'static str)>> = HashMap::new();
    for s in ocw_worldgen::junctions::steep_intervals(g, &js, &keep) {
        let cause = s.cause.key();
        steep
            .entry(s.edge)
            .or_default()
            .push((s.interval, s.grade, cause));
    }
    Side { js, steep }
}

/// Heights along edge `ei` from its end `k` (0: `from`), every metre up to
/// [`REACH_M`]: (drawn, profile).
fn heights(g: &RoadGraph, js: &JunctionSet, ei: usize, k: usize) -> (Vec<f64>, Vec<f64>) {
    let e = &g.edges[ei];
    let len = e.length_m();
    let mut drawn = Vec::new();
    let mut raw = Vec::new();
    let mut m = 0.0;
    while m <= REACH_M.min(len) + 1e-9 {
        let s = if k == 0 { m } else { len - m };
        let (p, _) = at(&e.points, s);
        let prof = e.height_at(s).unwrap_or(f64::NAN);
        let h = js.drawn_height_at_end(g, ei, k, s, p).unwrap_or(prof);
        drawn.push((h * 1000.0).round() / 1000.0);
        raw.push((prof * 1000.0).round() / 1000.0);
        m += 1.0;
    }
    (drawn, raw)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [live, j3, out] = &args[..] else {
        eprintln!("usage: junction_profiles LIVE.ocwpkg J3.ocwpkg OUT.jsonl");
        std::process::exit(2);
    };
    let (pl, pj) = (load(live), load(j3));
    let gl = pl.graph.clone().expect("road graph");
    let gj = pj.graph.clone().expect("road graph");
    let (sl, sj) = (side(&pl, &gl), side(&pj, &gj));
    // Live edges by (way, from vertex, to vertex) rounded to 0.1 m.
    let key = |g: &RoadGraph, ei: usize| {
        let e = &g.edges[ei];
        let r = |v: [f64; 2]| ((v[0] * 10.0).round() as i64, (v[1] * 10.0).round() as i64);
        (
            e.osm_way,
            r(g.vertices[e.from.0 as usize]),
            r(g.vertices[e.to.0 as usize]),
        )
    };
    let live_by: HashMap<_, usize> = (0..gl.edges.len()).map(|i| (key(&gl, i), i)).collect();
    let mut w = std::io::BufWriter::new(std::fs::File::create(out).expect("out"));
    let name = |g: &RoadGraph, ei: usize| g.edges[ei].name.clone().unwrap_or_default();
    let mut n = 0;
    for v in 0..gj.vertices.len() {
        let Some(patch) = sj.js.patch(&gj, v) else {
            continue;
        };
        if !patch.is_at(v) {
            continue;
        }
        let main_names: Vec<String> = patch
            .through
            .map(|t| t.iter().map(|&a| name(&gj, patch.arms[a].edge)).collect())
            .unwrap_or_default();
        let arm_names: Vec<String> = patch.arms.iter().map(|a| name(&gj, a.edge)).collect();
        for (ai, a) in patch.arms.iter().enumerate() {
            let ei = a.edge;
            let e = &gj.edges[ei];
            let k = if a.at_from { 0 } else { 1 };
            let rule = match patch.through {
                None => "blended",
                Some(t) if t.contains(&ai) => "main",
                _ if a.has_corner() => "crease",
                _ => "side",
            };
            let (dj, rj) = heights(&gj, &sj.js, ei, k);
            let len = e.length_m();
            let to_m = |iv: usize| {
                let s = iv as f64 * 5.0 + 2.5;
                if k == 0 { s } else { len - s }
            };
            let steep = |s: &Side, ei: usize| -> Vec<(f64, f64, &'static str)> {
                s.steep
                    .get(&ei)
                    .map(|v| {
                        v.iter()
                            .map(|&(iv, gr, c)| (to_m(iv), gr, c))
                            .filter(|x| x.0 <= REACH_M)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let live_e = live_by.get(&key(&gj, ei)).copied();
            let live = live_e.map(|le| {
                let (dl, rl) = heights(&gl, &sl.js, le, k);
                let ends = sl.js.ends(&gl, le);
                let arm = match &ends[k] {
                    Some(EndRule::Patch(p, i)) => {
                        let a = &p.arms[*i];
                        let r = match p.through {
                            None => "blended",
                            Some(t) if t.contains(i) => "main",
                            _ if a.has_corner() => "crease",
                            _ => "side",
                        };
                        serde_json::json!({"rule": r, "mouth_m": a.mouth_m, "blend_m": a.blend_m, "edge_m": a.edge_m})
                    }
                    _ => serde_json::Value::Null,
                };
                serde_json::json!({"edge": le, "drawn": dl, "profile": rl, "arm": arm, "steep": steep(&sl, le)})
            });
            let rec = serde_json::json!({
                "vertex": v,
                "pos": [patch.centre[0], patch.centre[2]],
                "centre_h": patch.centre[1],
                "members": patch.members.len(),
                "arms": patch.arms.len(),
                "arm": ai,
                "rule": rule,
                "edge": ei,
                "way": e.osm_way,
                "class": format!("{:?}", e.class),
                "width_m": e.width_m,
                "len_m": len,
                "name": name(&gj, ei),
                "main_names": main_names,
                "arm_names": arm_names,
                "dir": a.dir,
                "mouth_m": a.mouth_m,
                "blend_m": a.blend_m,
                "edge_m": a.edge_m,
                "edge_h": a.edge_h,
                "j3": {"drawn": dj, "profile": rj, "steep": steep(&sj, ei)},
                "live": live,
            });
            writeln!(w, "{rec}").expect("write");
            n += 1;
        }
    }
    eprintln!("{n} arms");
}
