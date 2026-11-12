//! Gradients of the real pilot package (Liechtenstein) as riders feel them
//! in free ride: along every edge, and through every rounded junction
//! corner of the ride path (spec §4.3, §5.2). Needs the pilot package in
//! `data/pilot/` (`scripts/fetch-pilot.sh` and a package build); skipped
//! with a message without it.

use std::path::PathBuf;
use std::sync::Arc;

use ocw_world::graph::RoadClass;
use ocw_world::{RidePath, RoadNetwork, Segment, Track, WorldPackage};

fn package() -> Option<(PathBuf, WorldPackage)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/pilot");
    let names = match std::env::var("OCW_PILOT_PKG") {
        Ok(n) => vec![n],
        Err(_) => vec!["liechtenstein.ocwpkg".to_string()],
    };
    for name in names {
        let p = dir.join(&name);
        if let Ok(bytes) = std::fs::read(&p) {
            let pkg = WorldPackage::from_bytes(&bytes).expect("readable package");
            if pkg.graph.is_some() {
                return Some((p, pkg));
            }
        }
    }
    None
}

/// Class gradient limit of the world builder (spec §4.3).
fn class_limit(class: RoadClass) -> f64 {
    match class {
        RoadClass::Trunk | RoadClass::Primary | RoadClass::Secondary => 0.15,
        RoadClass::Tertiary | RoadClass::Unclassified | RoadClass::Residential => 0.20,
        RoadClass::Cycleway => 0.20,
        RoadClass::Service | RoadClass::Track | RoadClass::Path => 0.25,
    }
}

#[test]
fn pilot_gradients_along_edges_and_through_corners() {
    let Some((path, pkg)) = package() else {
        eprintln!("skipped: no pilot package with a road graph in data/pilot");
        return;
    };
    let graph = pkg.graph.clone().expect("graph");
    let net = Arc::new(RoadNetwork::new(graph));
    let g = net.graph();
    eprintln!("{}: {} edges", path.display(), g.edges.len());
    let name = |e: usize| {
        g.edges[e]
            .name
            .clone()
            .unwrap_or_else(|| "(unnamed)".into())
    };

    // Along the edges: geometry point to point (the 5 m profile).
    let mut along: Vec<(f64, usize, [f64; 3])> = Vec::new();
    for e in 0..g.edges.len() {
        let pts = &net.geometry(e).points;
        let (mut worst, mut at) = (0.0f64, pts[0]);
        for w in pts.windows(2) {
            let run = ((w[1][0] - w[0][0]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt();
            if run > 0.5 {
                let grade = (w[1][1] - w[0][1]) / run;
                if grade.abs() > worst.abs() {
                    (worst, at) = (grade, w[0]);
                }
            }
        }
        along.push((worst, e, at));
    }
    along.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    eprintln!("steepest along edges:");
    for &(grade, e, p) in along.iter().take(8) {
        let edge = &g.edges[e];
        eprintln!(
            "  {:+6.1} %  edge {e} {:?} {} at x {:.0}, z {:.0} (limit {:.0} %, way {}, {:.1} m, ends {:.1}/{:.1} m, degree {}/{}, {} samples)",
            grade * 100.0,
            edge.class,
            name(e),
            p[0],
            p[2],
            class_limit(edge.class) * 100.0,
            edge.osm_way,
            edge.length_m(),
            edge.profile[0],
            edge.profile.last().unwrap(),
            net.edges_at(edge.from).len(),
            net.edges_at(edge.to).len(),
            edge.profile.len(),
        );
    }
    let over_along = along
        .iter()
        .filter(|&&(grade, e, _)| grade.abs() > class_limit(g.edges[e].class) + 0.01)
        .count();

    // Through every corner: a two-step path (in, out) sampled around the
    // junction, as the rider rides it.
    let mut corners: Vec<(f64, usize, usize, [f64; 3])> = Vec::new();
    for v in 0..g.vertices.len() {
        let at = ocw_world::graph::VertexId(v as u32);
        let edges = net.edges_at(at);
        for &a in edges {
            for &b in edges {
                if a == b {
                    continue;
                }
                let (la, lb) = (net.geometry(a).length(), net.geometry(b).length());
                if la < 1.0 || lb < 1.0 {
                    continue;
                }
                // Arrive at v on a, leave v on b.
                let a_fwd = g.edges[a].to == at;
                let b_fwd = g.edges[b].from == at;
                if (a_fwd && g.edges[a].from == at) || (b_fwd && g.edges[b].to == at) {
                    continue; // loops
                }
                let sa = Segment::Edge {
                    edge: a,
                    from_m: if a_fwd { 0.0 } else { la },
                    to_m: if a_fwd { la } else { 0.0 },
                };
                let sb = Segment::Edge {
                    edge: b,
                    from_m: if b_fwd { 0.0 } else { lb },
                    to_m: if b_fwd { lb } else { 0.0 },
                };
                let Ok(p) = RidePath::from_segments(net.clone(), &[sa, sb], 0.0, 0) else {
                    continue;
                };
                let j = p.step_start_m(1).expect("second step");
                let (lo, hi) = ((j - 15.0).max(0.0), (j + 15.0).min(p.end_m()));
                let mut s = lo;
                let (mut worst, mut wp) = (0.0f64, [0.0; 3]);
                while s <= hi {
                    let pt = p.sample(s);
                    if pt.grade.abs() > worst.abs() {
                        (worst, wp) = (pt.grade, pt.position);
                    }
                    s += 0.25;
                }
                corners.push((worst, a, b, wp));
            }
        }
    }
    corners.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    eprintln!("steepest through corners ({} corners):", corners.len());
    for &(grade, a, b, p) in corners.iter().take(8) {
        eprintln!(
            "  {:+6.1} %  edge {a} {} -> edge {b} {} at x {:.0}, z {:.0}",
            grade * 100.0,
            name(a),
            name(b),
            p[0],
            p[2]
        );
    }
    let worst_along = along.first().map_or(0.0, |a| a.0.abs());
    let worst_corner = corners.first().map_or(0.0, |c| c.0.abs());
    eprintln!(
        "max along edges {:.1} % ({over_along} edges above their class limit), max through corners {:.1} %",
        worst_along * 100.0,
        worst_corner * 100.0
    );
}
