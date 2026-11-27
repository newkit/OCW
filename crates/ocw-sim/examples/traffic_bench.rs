//! Benchmark and debug plots of the road traffic (spec 18E T3).
//!
//! ```text
//! cargo run --release -p ocw-sim --example traffic_bench -- \
//!     [--package FILE.ocwpkg] [--at X,Z] [--radius 1500] [--hour 17.5] \
//!     [--level busy] [--minutes 10] [--max 500] [--seed 1] \
//!     [--svg OUT.svg] [--ring OUT.svg] [--class-rules]
//! ```
//!
//! With `--package` the package's own rules drive the traffic (its
//! `TRFC`: one-ways, limits, access, lanes, signals and signs; spec 18E
//! T5), `--class-rules` the road class alone (T3's default) for a
//! comparison; it also prints how much of the time vehicles stand.
//!
//! Without `--package` it runs on a synthetic 20 × 20 grid of 150 m
//! blocks. It prints the vehicles, the step time (mean, 99th percentile,
//! max) and the plan changes per vehicle-minute. `--svg` draws the roads
//! and every vehicle's track over the run, coloured by speed; `--ring`
//! draws the space-time diagram of a lorry jam wave on a 2 km ring.

use std::time::Instant;

use ocw_sim::traffic::{
    Bubble, ClassRules, DemandTime, TrafficConfig, TrafficEvent, TrafficLevel, TrafficSim,
    VehicleClass,
};
use ocw_world::RoadNetwork;
use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph, VertexId};

struct Args {
    package: Option<String>,
    at: Option<[f64; 2]>,
    radius: f64,
    hour: f64,
    level: TrafficLevel,
    minutes: f64,
    max: usize,
    seed: u64,
    svg: Option<String>,
    ring: Option<String>,
    class_rules: bool,
}

fn args() -> Args {
    let mut a = Args {
        package: None,
        at: None,
        radius: 1500.0,
        hour: 17.5,
        level: TrafficLevel::Busy,
        minutes: 10.0,
        max: 500,
        seed: 1,
        svg: None,
        ring: None,
        class_rules: false,
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        let val = v.get(i + 1).cloned().unwrap_or_default();
        match v[i].as_str() {
            "--package" => a.package = Some(val),
            "--at" => {
                let p: Vec<f64> = val.split(',').filter_map(|x| x.parse().ok()).collect();
                a.at = (p.len() == 2).then(|| [p[0], p[1]]);
            }
            "--radius" => a.radius = val.parse().expect("radius"),
            "--hour" => a.hour = val.parse().expect("hour"),
            "--level" => a.level = TrafficLevel::parse(&val).expect("level"),
            "--minutes" => a.minutes = val.parse().expect("minutes"),
            "--max" => a.max = val.parse().expect("max"),
            "--seed" => a.seed = val.parse().expect("seed"),
            "--svg" => a.svg = Some(val),
            "--ring" => a.ring = Some(val),
            "--class-rules" => {
                a.class_rules = true;
                i += 1;
                continue;
            }
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    a
}

fn edge(i: usize, a: u32, b: u32, pts: Vec<[f64; 2]>, class: RoadClass) -> Edge {
    Edge {
        id: EdgeId(9000 + i as u64),
        from: VertexId(a),
        to: VertexId(b),
        points: pts,
        class,
        name: None,
        width_m: class.default_width_m(),
        bridge: false,
        tunnel: false,
        dead_end_m: None,
        boundary: false,
        oneway: Oneway::No,
        osm_way: i as i64,
        profile: Vec::new(),
    }
}

fn grid(n: u32, block: f64) -> RoadGraph {
    let mut vertices = Vec::new();
    for j in 0..n {
        for i in 0..n {
            vertices.push([f64::from(i) * block, f64::from(j) * block]);
        }
    }
    let class = |k: u32| match k % 4 {
        0 => RoadClass::Primary,
        2 => RoadClass::Tertiary,
        _ => RoadClass::Residential,
    };
    let mut edges = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let at = i + j * n;
            if i + 1 < n {
                let pts = vec![vertices[at as usize], vertices[at as usize + 1]];
                edges.push(edge(edges.len(), at, at + 1, pts, class(j)));
            }
            if j + 1 < n {
                let pts = vec![vertices[at as usize], vertices[(at + n) as usize]];
                edges.push(edge(edges.len(), at, at + n, pts, class(i)));
            }
        }
    }
    RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    }
}

fn load(path: &str) -> RoadNetwork {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let pkg = ocw_world::WorldPackage::skeleton_from_bytes(&bytes).expect("package");
    let graph = pkg.graph.clone().expect("a road graph");
    RoadNetwork::new(graph).with_traffic_of(&pkg)
}

/// The busiest junction (the most daily traffic of the roads meeting there).
fn busiest(net: &RoadNetwork, sim: &TrafficSim) -> [f64; 2] {
    let g = net.graph();
    let best = (0..g.vertices.len())
        .max_by(|&a, &b| {
            let f = |v: usize| -> f64 {
                net.edges_at(VertexId(v as u32))
                    .iter()
                    .map(|&e| sim.rule(e).aadt)
                    .sum()
            };
            f(a).total_cmp(&f(b))
        })
        .expect("vertices");
    g.vertices[best]
}

fn speed_colour(v: f64) -> String {
    // Red (standing) → yellow → green (≥ 20 m/s).
    let t = (v / 20.0).clamp(0.0, 1.0);
    let (r, g) = if t < 0.5 {
        (230.0, 60.0 + 340.0 * t)
    } else {
        (230.0 - 400.0 * (t - 0.5), 230.0)
    };
    format!("rgb({:.0},{:.0},60)", r.max(30.0), g.min(230.0))
}

fn main() {
    let a = args();
    if let Some(out) = &a.ring {
        ring_plot(out);
        return;
    }
    let net = match &a.package {
        Some(p) => load(p),
        None => RoadNetwork::new(grid(20, 150.0)),
    };
    let cfg = TrafficConfig {
        seed: a.seed,
        level: a.level,
        max_vehicles: a.max,
        ..TrafficConfig::default()
    };
    let rules: Box<dyn ocw_sim::traffic::EdgeRules + Send + Sync> = if a.class_rules {
        Box::new(ClassRules)
    } else {
        let r = ocw_sim::traffic::package::PackageRules::new(&net);
        println!(
            "rules: the package's ({}), {} junctions with signals, {} signed arrivals",
            if r.from_data() {
                "TRFC"
            } else {
                "implied by class"
            },
            r.signalled(),
            r.controls()
        );
        Box::new(r)
    };
    let mut sim = TrafficSim::new(&net, rules, cfg);
    let center = a.at.unwrap_or_else(|| {
        if a.package.is_some() {
            busiest(&net, &sim)
        } else {
            [1425.0, 1425.0]
        }
    });
    let bubble = Bubble {
        center,
        radius: a.radius,
    };
    let time = DemandTime::weekday_at(a.hour);
    let ticks = (a.minutes * 60.0 * 50.0) as u64;
    let mut times: Vec<f64> = Vec::with_capacity(ticks as usize);
    let mut counts = 0u64;
    let mut changes = 0u64;
    let mut tracks: Vec<([f64; 2], f64)> = Vec::new();
    let mut near_counts = 0u64;
    let (mut standing, mut sampled) = (0u64, 0u64);
    let warm = 3000;
    for t in 0..ticks + warm {
        let start = Instant::now();
        sim.step(&net, &[bubble], &[], &time.plus_seconds((t / 50) as u32));
        let us = start.elapsed().as_secs_f64() * 1e6;
        let ev = sim.drain_events();
        if t >= warm {
            times.push(us);
            counts += sim.len() as u64;
            changes += ev
                .iter()
                .filter(|e| matches!(e, TrafficEvent::Change(_)))
                .count() as u64;
            if t.is_multiple_of(50) {
                for (_, v) in sim.speeds() {
                    sampled += 1;
                    if v < 0.3 {
                        standing += 1;
                    }
                }
                for v in sim.views(&net) {
                    let (dx, dz) = (v.xz[0] - center[0], v.xz[1] - center[1]);
                    if dx * dx + dz * dz < 300.0 * 300.0 {
                        near_counts += 1;
                    }
                    if a.svg.is_some() && t.is_multiple_of(250) {
                        tracks.push((v.xz, v.speed));
                    }
                }
            }
        }
    }
    let n = times.len() as f64;
    let mean = times.iter().sum::<f64>() / n;
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    let p99 = sorted[((n * 0.99) as usize).min(sorted.len() - 1)];
    let max = sorted.last().copied().unwrap_or(0.0);
    let vehicles = counts as f64 / n;
    let near = near_counts as f64 / (n / 50.0);
    let per_min = changes as f64 / (counts as f64 / 50.0 / 60.0);
    println!(
        "network: {} edges; bubble at ({:.0}, {:.0}) r {:.0} m; level {}; hour {}",
        net.graph().edges.len(),
        center[0],
        center[1],
        a.radius,
        a.level.name(),
        a.hour
    );
    println!(
        "vehicles: {vehicles:.0} on average in the bubble, {near:.0} within 300 m; spawned {}, despawned {}",
        sim.stats().spawned,
        sim.stats().despawned
    );
    println!(
        "step (50 Hz): mean {mean:.0} us, p99 {p99:.0} us, max {max:.0} us; {:.2} us per vehicle",
        mean / vehicles.max(1.0)
    );
    println!("plan changes: {per_min:.2} per vehicle-minute");
    println!(
        "standing: {:.1} % of the vehicle-seconds; {} vehicles stand at the end",
        100.0 * standing as f64 / sampled.max(1) as f64,
        sim.speeds().iter().filter(|(_, v)| *v < 0.3).count()
    );
    // Where vehicles stand at the end: the five vertices with the most,
    // and the control each arm has there.
    let diag = ocw_sim::traffic::package::PackageRules::new(&net);
    let mut at: std::collections::BTreeMap<u32, (usize, f64)> = Default::default();
    for (vx, d, v) in sim.at_junctions(&net) {
        if v < 0.3 {
            let e = at.entry(vx.0).or_insert((0, f64::INFINITY));
            e.0 += 1;
            e.1 = e.1.min(d);
        }
    }
    let mut top: Vec<(u32, (usize, f64))> = at.into_iter().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1.0));
    for (vx, (n, d)) in top.iter().take(5) {
        let p = net.graph().vertices[*vx as usize];
        let arms: Vec<String> = net
            .edges_at(VertexId(*vx))
            .iter()
            .map(|&e| {
                let fwd = net.edge(e).to == VertexId(*vx);
                use ocw_sim::traffic::EdgeRules as _;
                format!(
                    "{:?}/{:?}/lanes {:?}",
                    net.edge(e).class,
                    diag.node_control(&net, VertexId(*vx), e, fwd),
                    sim.rule(e).lanes
                )
            })
            .collect();
        println!(
            "  {n} standing before vertex {vx} at ({:.0}, {:.0}), nearest {d:.0} m: {}",
            p[0],
            p[1],
            arms.join(", ")
        );
    }
    if let Some(out) = &a.svg {
        write_map(out, &net, &sim, bubble, &tracks);
        println!("wrote {out}");
    }
}

fn write_map(
    out: &str,
    net: &RoadNetwork,
    sim: &TrafficSim,
    b: Bubble,
    tracks: &[([f64; 2], f64)],
) {
    let r = b.radius + 200.0;
    let (x0, z0) = (b.center[0] - r, b.center[1] - r);
    let size = 2.0 * r;
    let px = 1600.0;
    let k = px / size;
    let mut s = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{px}' height='{px}' viewBox='0 0 {px} {px}'>\n<rect width='100%' height='100%' fill='#f4f2ec'/>\n"
    );
    for (i, e) in net.graph().edges.iter().enumerate() {
        let bb = e
            .points
            .iter()
            .any(|p| (p[0] - b.center[0]).abs() < r && (p[1] - b.center[1]).abs() < r);
        if !bb {
            continue;
        }
        let rule = sim.rule(i);
        let (w, c) = if rule.access.is_none() {
            (1.0, "#b8c8b0")
        } else {
            (1.0 + f64::from(rule.rank) * 0.6, "#8a8a8a")
        };
        let pts: Vec<String> = e
            .points
            .iter()
            .map(|p| format!("{:.1},{:.1}", (p[0] - x0) * k, (p[1] - z0) * k))
            .collect();
        s += &format!(
            "<polyline points='{}' fill='none' stroke='{c}' stroke-width='{w:.1}'/>\n",
            pts.join(" ")
        );
    }
    for (p, v) in tracks {
        s += &format!(
            "<circle cx='{:.1}' cy='{:.1}' r='1.6' fill='{}' fill-opacity='0.7'/>\n",
            (p[0] - x0) * k,
            (p[1] - z0) * k,
            speed_colour(*v)
        );
    }
    for v in sim.views(net) {
        let shape = match v.class {
            VehicleClass::Lorry | VehicleClass::Bus => 4.5,
            VehicleClass::Motorbike => 2.5,
            _ => 3.5,
        };
        s += &format!(
            "<circle cx='{:.1}' cy='{:.1}' r='{shape}' fill='#1d3557' stroke='white' stroke-width='0.8'/>\n",
            (v.xz[0] - x0) * k,
            (v.xz[1] - z0) * k
        );
    }
    s += &format!(
        "<circle cx='{:.1}' cy='{:.1}' r='{:.1}' fill='none' stroke='#e76f51' stroke-width='2' stroke-dasharray='8 6'/>\n",
        (b.center[0] - x0) * k,
        (b.center[1] - z0) * k,
        b.radius * k
    );
    s += &format!(
        "<circle cx='{:.1}' cy='{:.1}' r='6' fill='#e76f51'/>\n",
        (b.center[0] - x0) * k,
        (b.center[1] - z0) * k
    );
    s += "<text x='16' y='30' font-family='sans-serif' font-size='20' fill='#333'>Traffic core T3: vehicle tracks every 5 s, red = standing, green = 20 m/s; dashed = the rider's bubble</text>\n</svg>\n";
    std::fs::write(out, s).expect("write svg");
}

/// The space-time diagram of a lorry jam wave on a 2 km ring.
fn ring_plot(out: &str) {
    let n = 40u32;
    let length = 2000.0;
    let rr = length / (2.0 * std::f64::consts::PI);
    let vertices: Vec<[f64; 2]> = (0..n)
        .map(|k| {
            let t = f64::from(k) / f64::from(n) * 2.0 * std::f64::consts::PI;
            [rr * t.cos(), rr * t.sin()]
        })
        .collect();
    let edges: Vec<Edge> = (0..n)
        .map(|k| {
            let b = (k + 1) % n;
            let mut e = edge(
                k as usize,
                k,
                b,
                vec![vertices[k as usize], vertices[b as usize]],
                RoadClass::Secondary,
            );
            e.oneway = Oneway::Forward;
            e
        })
        .collect();
    let net = RoadNetwork::new(RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    });
    let cfg = TrafficConfig {
        seed: 11,
        step_ms: 100,
        level: TrafficLevel::Off,
        ..TrafficConfig::default()
    };
    let mut sim = TrafficSim::new(&net, Box::new(ClassRules), cfg);
    let bubble = Bubble {
        center: [0.0, 0.0],
        radius: 1000.0,
    };
    let time = DemandTime::weekday_at(8.0);
    let edge_len = net.geometry(0).length();
    for k in 0..70 {
        let d = length * f64::from(k) / 70.0;
        let e = ((d / edge_len) as usize).min(39);
        let route: Vec<(usize, bool)> = (0..3).map(|j| ((e + j) % 40, true)).collect();
        sim.spawn_on(
            &net,
            VehicleClass::Lorry,
            &route,
            d - e as f64 * edge_len,
            8.0,
            0,
        );
    }
    let (w, h) = (1000.0, 1200.0);
    let total = 12_000u32;
    let mut s = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}'>\n<rect width='100%' height='100%' fill='white'/>\n"
    );
    for t in 0..total {
        if t == 50 {
            let first = sim.speeds()[0].0;
            sim.set_speed(first, 0.0);
        }
        if t == 7_000 {
            let ids: Vec<u32> = sim.speeds().iter().map(|s| s.0).collect();
            for (k, id) in ids.iter().enumerate() {
                if k % 3 != 0 {
                    sim.remove(*id);
                }
            }
        }
        sim.step(&net, &[bubble], &[], &time);
        if t.is_multiple_of(20) {
            let y = f64::from(t) / f64::from(total) * (h - 60.0) + 40.0;
            for v in sim.views(&net) {
                let along = v.edge as f64 * edge_len + v.s;
                s += &format!(
                    "<rect x='{:.1}' y='{y:.1}' width='2' height='2' fill='{}'/>\n",
                    along / length * w,
                    speed_colour(v.speed)
                );
            }
        }
    }
    s += "<text x='10' y='24' font-family='sans-serif' font-size='12'>Lorries on a 2 km ring, 35/km: time downwards (20 min), position across; red = standing. Two thirds leave after 11 min 40 s and the wave dissolves.</text>\n</svg>\n";
    std::fs::write(out, s).expect("write svg");
    println!("wrote {out}");
}
