//! The road kinks (`road.vertical.kink`: a change of grade over 8 points
//! between two 5 m intervals) one package has and another, built from the
//! same road graph, has not – by what the junction at the nearer end of the
//! edge is in the second package (spec 4.8.8, J8: whether a junction's arm
//! runs on past a way split there).
//!
//! ```text
//! cargo run -p ocw-worldgen --release --example kink_diff -- BEFORE.ocwpkg AFTER.ocwpkg
//! ```

use std::collections::{BTreeMap, HashSet};

use ocw_world::WorldPackage;
use ocw_world::graph::PROFILE_STEP_M;
use ocw_world::junction::JunctionSet;

fn kinks(g: &ocw_world::graph::RoadGraph) -> HashSet<(usize, usize)> {
    let mut out = HashSet::new();
    for (ei, e) in g.edges.iter().enumerate() {
        let p = &e.profile;
        let len = e.length_m();
        if p.len() < 3 {
            continue;
        }
        let d: Vec<f64> = (0..p.len())
            .map(|i| (i as f64 * PROFILE_STEP_M).min(len))
            .collect();
        let mut prev: Option<f64> = None;
        for i in 0..p.len() - 1 {
            let ds = d[i + 1] - d[i];
            if ds < 1.0 {
                continue;
            }
            let grade = (p[i + 1] - p[i]) / ds;
            if let Some(pg) = prev
                && (grade - pg).abs() > 0.08
            {
                out.insert((ei, i));
            }
            prev = Some(grade);
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a, b] = &args[..] else {
        eprintln!("usage: kink_diff BEFORE.ocwpkg AFTER.ocwpkg");
        std::process::exit(2);
    };
    let load =
        |p: &str| WorldPackage::from_bytes(&std::fs::read(p).expect("package")).expect("pkg");
    let (pa, pb) = (load(a), load(b));
    let (ga, gb) = (
        pa.graph.clone().expect("graph"),
        pb.graph.clone().expect("graph"),
    );
    assert_eq!(ga.edges.len(), gb.edges.len(), "the same road graph");
    let js = match pb.junctions.as_ref() {
        Some(j) => JunctionSet::with_index(&gb, j.index_for(&gb)),
        None => JunctionSet::new(&gb),
    };
    let (ka, kb) = (kinks(&ga), kinks(&gb));
    let mut by: BTreeMap<String, [usize; 2]> = BTreeMap::new();
    let mut note = |set: &HashSet<(usize, usize)>, k: usize| {
        for &(ei, i) in set {
            let e = &gb.edges[ei];
            let len = e.length_m();
            let s = (i as f64 * PROFILE_STEP_M).min(len);
            let v = if s < len / 2.0 { e.from.0 } else { e.to.0 } as usize;
            let what = match js.patch(&gb, v) {
                Some(p) if !p.run_on.is_empty() => {
                    if p.inner.contains(&ei) {
                        "run on: the edge run on past".to_string()
                    } else if p.arms.iter().any(|a| a.edge == ei && !a.lead.is_empty()) {
                        "run on: the arm beyond the split".to_string()
                    } else {
                        "run on: another arm".to_string()
                    }
                }
                Some(p) if p.is_cluster() => "cluster".into(),
                Some(p) if p.through.is_some() => "patch with a through road".into(),
                Some(_) => "blended patch".into(),
                None => "no patch".into(),
            };
            by.entry(what).or_default()[k] += 1;
        }
    };
    let gone: HashSet<_> = ka.difference(&kb).copied().collect();
    let new: HashSet<_> = kb.difference(&ka).copied().collect();
    note(&gone, 0);
    note(&new, 1);
    println!(
        "kinks {} -> {} (gone {}, new {})",
        ka.len(),
        kb.len(),
        gone.len(),
        new.len()
    );
    for (k, [g, n]) in by {
        println!("{k}: gone {g}, new {n}");
    }
}
