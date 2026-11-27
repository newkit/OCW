//! The road traffic on a real world package for a simulated hour (spec
//! 18H, 18E T3 / T5): no permanent queue, no gridlock. Slow (a minute or
//! two in release) and it needs a package, so it is ignored by default:
//!
//! ```text
//! OCW_TRAFFIC_PACKAGE=~/src/OCW/data/traffic-build/liechtenstein.ocwpkg \
//!     cargo test --release -p ocw-sim --test traffic_package -- --ignored --nocapture
//! ```
//!
//! On the Liechtenstein package at busy traffic at 17:30 the old junction
//! rules formed a permanent queue at vertices 9 and 3710 of the main road
//! (41 % of the vehicle time standing after 10 minutes, 86 % over the
//! hour); the bubble is the one `traffic_bench` puts on its busiest
//! junction.

use std::collections::{BTreeMap, BTreeSet};

use ocw_sim::traffic::{
    Bubble, DemandTime, EdgeRules, TrafficConfig, TrafficLevel, TrafficSim, WorldRules,
};
use ocw_world::RoadNetwork;

fn load(path: &str) -> RoadNetwork {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let pkg = ocw_world::WorldPackage::skeleton_from_bytes(&bytes).expect("package");
    let graph = pkg.graph.clone().expect("a road graph");
    RoadNetwork::new(graph).with_traffic_of(&pkg)
}

/// Vehicles in a closed cycle of waits (each waits for the vehicle it
/// yields to, the one in its full exit or in the junction, else the one
/// ahead, and the chain comes back to it).
fn gridlocked(sim: &mut TrafficSim, net: &RoadNetwork) -> usize {
    let waits = sim.waits(net);
    let next: BTreeMap<u32, u32> = waits
        .iter()
        .filter_map(|w| w.blocker.or(w.behind).map(|b| (w.id, b)))
        .collect();
    let mut stuck = BTreeSet::new();
    for w in &waits {
        let mut seen = Vec::new();
        let mut cur = w.id;
        while let Some(&n) = next.get(&cur) {
            if let Some(at) = seen.iter().position(|&x| x == cur) {
                stuck.extend(seen[at..].iter().copied());
                break;
            }
            seen.push(cur);
            cur = n;
        }
    }
    stuck.len()
}

fn run(net: &RoadNetwork, rules: Box<dyn EdgeRules + Send + Sync>, what: &str) {
    let cfg = TrafficConfig {
        seed: 1,
        level: TrafficLevel::Busy,
        max_vehicles: 500,
        ..TrafficConfig::default()
    };
    let mut sim = TrafficSim::new(net, rules, cfg);
    // The bench's bubble: 1.5 km around the busiest junction.
    let g = net.graph();
    let busiest = (0..g.vertices.len())
        .max_by(|&a, &b| {
            let f = |v: usize| -> f64 {
                net.edges_at(ocw_world::graph::VertexId(v as u32))
                    .iter()
                    .map(|&e| sim.rule(e).aadt)
                    .sum()
            };
            f(a).total_cmp(&f(b))
        })
        .expect("vertices");
    let bubble = Bubble {
        center: g.vertices[busiest],
        radius: 1500.0,
    };
    let time = DemandTime::weekday_at(17.5);
    let hz = 1000 / u64::from(sim.config().step_ms);
    let mut windows = [(0u64, 0u64); 6];
    for t in 0..(61 * 60 * hz) {
        sim.step(net, &[bubble], &[], &time.plus_seconds((t / hz) as u32));
        if t >= 60 * hz && t % hz == 0 {
            let w = (((t / hz) - 60) / 600) as usize;
            for (_, v) in sim.speeds() {
                windows[w.min(5)].1 += 1;
                if v < 0.3 {
                    windows[w.min(5)].0 += 1;
                }
            }
        }
    }
    let shares: Vec<f64> = windows
        .iter()
        .map(|(s, n)| *s as f64 / (*n).max(1) as f64)
        .collect();
    let standing = sim.speeds().iter().filter(|s| s.1 < 0.3).count();
    let stuck = gridlocked(&mut sim, net);
    eprintln!(
        "{what}: standing per 10 min {:?} %, {standing} of {} standing at the end, {stuck} in a gridlock",
        shares
            .iter()
            .map(|s| (1000.0 * s).round() / 10.0)
            .collect::<Vec<_>>(),
        sim.len()
    );
    assert_eq!(stuck, 0, "{what}: no gridlock after an hour");
    for (k, s) in shares.iter().enumerate() {
        assert!(
            *s < 0.12,
            "{what}: {:.1} % standing in minutes {}–{}",
            100.0 * s,
            10 * k,
            10 * k + 10
        );
    }
    assert!(
        standing * 5 < sim.len(),
        "{what}: {standing} of {} stand at the end",
        sim.len()
    );
}

#[test]
#[ignore = "slow; needs OCW_TRAFFIC_PACKAGE"]
fn no_permanent_queue_in_an_hour_on_a_real_package() {
    let Ok(path) = std::env::var("OCW_TRAFFIC_PACKAGE") else {
        eprintln!("OCW_TRAFFIC_PACKAGE not set: skipped");
        return;
    };
    let net = load(&path);
    run(&net, Box::new(WorldRules::new(&net)), "package rules");
    run(&net, Box::new(ocw_sim::traffic::ClassRules), "class rules");
}
