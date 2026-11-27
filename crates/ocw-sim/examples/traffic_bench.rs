//! Benchmark and debug plots of the road traffic (spec 18E T3, T4).
//!
//! ```text
//! cargo run --release -p ocw-sim --example traffic_bench -- \
//!     [--riders N] [--around DIR] \
//!     [--package FILE.ocwpkg] [--at X,Z] [--radius 1500] [--hour 17.5] \
//!     [--level busy] [--minutes 10] [--max 500] [--seed 1] \
//!     [--svg OUT.svg] [--ring OUT.svg]
//! ```
//!
//! Without `--package` it runs on a synthetic 20 × 20 grid of 150 m
//! blocks. It prints the vehicles, the step time (mean, 99th percentile,
//! max) and the plan changes per vehicle-minute. `--svg` draws the roads
//! and every vehicle's track over the run, coloured by speed; `--ring`
//! draws the space-time diagram of a lorry jam wave on a 2 km ring.
//! `--riders N` puts N riders on the roads nearest the centre (T4:
//! waiting, passing, the pairs and the gusts are part of the measured
//! step); `--around DIR` runs T4's stadium for each driver manner and the
//! lunatics and writes the passing-distance histograms and a top-down of
//! one overtake into DIR.

use std::time::Instant;

use ocw_sim::traffic::{
    Bubble, ClassRules, DemandTime, DriverManners, PassRecord, RiderEvent, RoadUser, TrafficConfig,
    TrafficEvent, TrafficLevel, TrafficSim, VehicleClass,
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
    riders: usize,
    around: Option<String>,
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
        riders: 0,
        around: None,
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
            "--riders" => a.riders = val.parse().expect("riders"),
            "--around" => a.around = Some(val),
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
    RoadNetwork::new(graph)
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
    if let Some(dir) = &a.around {
        around_plots(dir);
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
    let mut sim = TrafficSim::new(&net, Box::new(ClassRules), cfg);
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
    let warm = 3000;
    // Riders on the edges nearest the centre, each riding its edge to the
    // end and starting again (T4).
    let mut near: Vec<(f64, usize)> = (0..net.graph().edges.len())
        .filter(|&e| !sim.rule(e).access.is_none())
        .map(|e| {
            let p = net.geometry(e).point_at(net.geometry(e).length() / 2.0);
            let (dx, dz) = (p[0] - center[0], p[2] - center[1]);
            (dx * dx + dz * dz, e)
        })
        .collect();
    near.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut riders: Vec<RoadUser> = near
        .iter()
        .take(a.riders)
        .enumerate()
        .map(|(k, &(_, e))| RoadUser {
            id: k as u32 + 1,
            edge: e,
            s: 0.0,
            forward: k % 2 == 0,
            speed: 5.0 + (k % 5) as f64,
            lateral: 1.0 + 0.2 * (k % 4) as f64,
            ..RoadUser::default()
        })
        .collect();
    let mut rider_d = vec![0.0f64; riders.len()];
    let mut passes = 0usize;
    for t in 0..ticks + warm {
        for (r, d) in riders.iter_mut().zip(rider_d.iter_mut()) {
            let len = net.geometry(r.edge).length();
            *d = (*d + r.speed * 0.02) % len;
            r.s = if r.forward { *d } else { len - *d };
        }
        let start = Instant::now();
        sim.step(
            &net,
            &[bubble],
            &riders,
            &time.plus_seconds((t / 50) as u32),
        );
        passes += sim
            .drain_rider_events()
            .iter()
            .filter(|e| matches!(e, RiderEvent::Passed(_)))
            .count();
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
    if a.riders > 0 {
        println!("riders: {}; overtakes: {passes}", a.riders);
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

/// T4's stadium (the unit tests' loop): two 4 km straights, bends of
/// 150 m radius; unclassified in town for the first 40 % of a straight,
/// secondary outside. The network and the edges' cumulative starts.
fn stadium() -> (RoadNetwork, Vec<f64>) {
    let (l, r, seg) = (4000.0, 150.0, 250.0);
    let n = (l / seg) as usize;
    let class = |k: usize| {
        if (k as f64) < 0.4 * n as f64 {
            RoadClass::Unclassified
        } else {
            RoadClass::Secondary
        }
    };
    let mut pts: Vec<([f64; 2], RoadClass)> = Vec::new();
    for k in 0..n {
        pts.push(([k as f64 * seg, 0.0], class(k)));
    }
    for k in 0..12 {
        let t = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / 12.0;
        pts.push(([l + r * t.cos(), r + r * t.sin()], RoadClass::Secondary));
    }
    for k in 0..n {
        pts.push(([l - k as f64 * seg, 2.0 * r], class(k)));
    }
    for k in 0..12 {
        let t = std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / 12.0;
        pts.push(([r * t.cos(), r + r * t.sin()], RoadClass::Secondary));
    }
    let m = pts.len();
    let vertices: Vec<[f64; 2]> = pts.iter().map(|p| p.0).collect();
    let edges = (0..m)
        .map(|k| {
            let b = (k + 1) % m;
            edge(
                k,
                k as u32,
                b as u32,
                vec![vertices[k], vertices[b]],
                pts[k].1,
            )
        })
        .collect();
    let net = RoadNetwork::new(RoadGraph {
        vertices,
        edges,
        boundary_vertices: Default::default(),
    });
    let mut cum = vec![0.0];
    for e in 0..m {
        let c = cum[e] + net.geometry(e).length();
        cum.push(c);
    }
    (net, cum)
}

/// 80 riders in groups on the stadium, as in the unit tests: one speed a
/// direction (7.5 / 6 m/s), 1.8 – 2.2 m right of the centreline:
/// (position along the loop, speed, lateral, forward).
fn stadium_riders(total: f64) -> Vec<(f64, f64, f64, bool)> {
    let mut h: u64 = 0x2545_F491_4F6C_DD1D;
    let mut u = || {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        (h >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut out = Vec::new();
    let mut g = 0;
    while out.len() < 80 {
        let size = 1 + (u() * 4.0) as usize;
        let fwd = g % 2 == 0;
        let (mut p, speed, lateral) = (u() * total, if fwd { 7.5 } else { 6.0 }, 1.8 + 0.4 * u());
        for _ in 0..size {
            out.push((p, speed, lateral, fwd));
            let gap = 1.75 + 1.0 + 1.5 * u();
            p = (p - if fwd { gap } else { -gap }).rem_euclid(total);
        }
        g += 1;
    }
    out
}

fn loop_user(id: u32, r: (f64, f64, f64, bool), cum: &[f64]) -> RoadUser {
    let e = cum
        .partition_point(|&c| c <= r.0)
        .saturating_sub(1)
        .min(cum.len() - 2);
    RoadUser {
        id,
        edge: e,
        s: r.0 - cum[e],
        forward: r.3,
        speed: r.1,
        lateral: r.2,
        ..RoadUser::default()
    }
}

/// One frame of the overtake's history: the tick, the vehicles (id,
/// position) and the riders' positions.
type Frame = (u64, Vec<(u32, [f64; 2])>, Vec<[f64; 2]>);

/// The passing-distance histograms per manner and the lunatics, and a
/// top-down of one overtake (T4).
fn around_plots(dir: &str) {
    std::fs::create_dir_all(dir).expect("dir");
    let (net, cum) = stadium();
    let total = *cum.last().expect("loop");
    let cases = [
        ("polite", DriverManners::Polite, TrafficLevel::Busy),
        ("mostly-polite", DriverManners::Honking, TrafficLevel::Busy),
        ("some-realism", DriverManners::Realistic, TrafficLevel::Busy),
        ("lunatic", DriverManners::Polite, TrafficLevel::Lunatic),
    ];
    let bubble = Bubble {
        center: [2000.0, 150.0],
        radius: 2400.0,
    };
    let time = DemandTime::weekday_at(10.0);
    for (name, manners, level) in cases {
        let mut sim = TrafficSim::new(
            &net,
            Box::new(ClassRules),
            TrafficConfig {
                seed: 3,
                level,
                manners,
                collisions: true,
                ..TrafficConfig::default()
            },
        );
        let mut riders = stadium_riders(total);
        let mut all: Vec<PassRecord> = Vec::new();
        let (mut contacts, mut honks) = (0, 0);
        let mut t = 0u64;
        // One overtake's top-down: the first pass into the oncoming half
        // on the bottom straight.
        let mut hist: Vec<Frame> = Vec::new();
        let mut picked = false;
        while all.len() < 3000 && t < 3 * 3600 * 50 {
            let users: Vec<RoadUser> = riders
                .iter()
                .enumerate()
                .map(|(k, r)| loop_user(k as u32 + 1, *r, &cum))
                .collect();
            sim.step(&net, &[bubble], &users, &time);
            let views = sim.views(&net);
            for e in sim.drain_rider_events() {
                match e {
                    RiderEvent::Passed(p) => {
                        let on_straight = views
                            .iter()
                            .any(|v| v.id == p.vehicle && v.xz[1].abs() < 10.0 && v.xz[0] > 400.0);
                        if !picked && p.oncoming_half && hist.len() >= 1200 && on_straight {
                            picked = true;
                            write_overtake(&format!("{dir}/overtake-{name}.svg"), &hist, &p);
                        }
                        all.push(p);
                    }
                    RiderEvent::Contact(_) => contacts += 1,
                    RiderEvent::Honk { .. } => honks += 1,
                    _ => {}
                }
            }
            if !picked {
                let vs: Vec<(u32, [f64; 2])> = views.iter().map(|v| (v.id, v.xz)).collect();
                let rs: Vec<[f64; 2]> = users
                    .iter()
                    .map(|u| {
                        // The rider's place: the centreline plus its lateral.
                        let g = net.geometry(u.edge);
                        let len = g.length();
                        let p = g.point_at(u.s);
                        let q = g.point_at((u.s + 1.0).min(len));
                        let o = g.point_at((u.s - 1.0).max(0.0));
                        let (hx, hz) = (q[0] - o[0], q[2] - o[2]);
                        let n = (hx * hx + hz * hz).sqrt().max(1e-9);
                        let dir = if u.forward { 1.0 } else { -1.0 };
                        let (hx, hz) = (hx / n * dir, hz / n * dir);
                        [p[0] - hz * u.lateral, p[2] + hx * u.lateral]
                    })
                    .collect();
                hist.push((t, vs, rs));
                if hist.len() > 1200 {
                    hist.remove(0);
                }
            }
            for r in &mut riders {
                let dir = if r.3 { 1.0 } else { -1.0 };
                r.0 = (r.0 + dir * r.1 * 0.02).rem_euclid(total);
            }
            t += 1;
        }
        let close = all
            .iter()
            .filter(|p| p.clearance_m < p.legal_m - 1e-6)
            .count();
        let min = all
            .iter()
            .map(|p| p.clearance_m)
            .fold(f64::INFINITY, f64::min);
        println!(
            "{name}: {} passes in {:.0} min, {close} closer than the law, closest {min:.2} m, {contacts} contacts, {honks} honks",
            all.len(),
            t as f64 / 50.0 / 60.0
        );
        write_histogram(
            &format!("{dir}/passes-{name}.svg"),
            name,
            &all,
            contacts,
            honks,
        );
    }
}

fn write_histogram(out: &str, name: &str, all: &[PassRecord], contacts: usize, honks: usize) {
    let (w, h) = (900.0, 520.0);
    let (x0, x1) = (0.0, 4.0);
    let bins = 80;
    let mut town = vec![0usize; bins];
    let mut open = vec![0usize; bins];
    for p in all {
        let b = ((p.clearance_m.clamp(x0, x1 - 1e-9) - x0) / (x1 - x0) * bins as f64) as usize;
        if p.legal_m < 1.75 {
            town[b] += 1;
        } else {
            open[b] += 1;
        }
    }
    let top = town
        .iter()
        .zip(&open)
        .map(|(a, b)| a + b)
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let (l, r, t, bt) = (70.0, 20.0, 60.0, 60.0);
    let px = |x: f64| l + (x - x0) / (x1 - x0) * (w - l - r);
    let py = |n: f64| h - bt - n / top * (h - t - bt);
    let mut s = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}'>\n<rect width='100%' height='100%' fill='white'/>\n"
    );
    let bw = (w - l - r) / bins as f64;
    for k in 0..bins {
        let x = l + k as f64 * bw;
        let (a, b) = (town[k] as f64, open[k] as f64);
        s += &format!(
            "<rect x='{x:.1}' y='{:.1}' width='{:.1}' height='{:.1}' fill='#2a9d8f'/>\n",
            py(a),
            bw - 1.0,
            h - bt - py(a)
        );
        s += &format!(
            "<rect x='{x:.1}' y='{:.1}' width='{:.1}' height='{:.1}' fill='#e9c46a'/>\n",
            py(a + b),
            bw - 1.0,
            py(a) - py(a + b)
        );
    }
    for (legal, c) in [(1.5, "#2a9d8f"), (2.0, "#b08900"), (1.0, "#e76f51")] {
        s += &format!(
            "<line x1='{0:.1}' x2='{0:.1}' y1='{t}' y2='{1}' stroke='{c}' stroke-width='2' stroke-dasharray='6 4'/>\n",
            px(legal),
            h - bt
        );
    }
    for k in 0..=8 {
        let x = f64::from(k) * 0.5;
        s += &format!(
            "<text x='{:.1}' y='{}' font-family='sans-serif' font-size='13' text-anchor='middle'>{x:.1}</text>\n",
            px(x),
            h - bt + 20.0
        );
    }
    let close = all
        .iter()
        .filter(|p| p.clearance_m < p.legal_m - 1e-6)
        .count();
    s += &format!(
        "<text x='{l}' y='28' font-family='sans-serif' font-size='18'>Traffic T4, {name}: lateral gap of {} overtakes (vehicle side to handlebar, m)</text>\n",
        all.len()
    );
    s += &format!(
        "<text x='{l}' y='48' font-family='sans-serif' font-size='13' fill='#444'>green: in town (legal 1.5 m), yellow: outside (legal 2 m); dashed: 1.5 / 2 m and the 1 m floor. {close} closer than the law, {contacts} contacts, {honks} honks</text>\n"
    );
    s += &format!(
        "<text x='{:.1}' y='{}' font-family='sans-serif' font-size='13' text-anchor='middle'>gap (m)</text>\n</svg>\n",
        w / 2.0,
        h - 15.0
    );
    std::fs::write(out, s).expect("write svg");
}

/// A top-down of one overtake on the stadium's bottom straight, the
/// lateral scale stretched ×8 so the gap shows.
fn write_overtake(out: &str, hist: &[Frame], p: &PassRecord) {
    let last = hist.last().expect("history");
    let Some(end) = last.1.iter().find(|v| v.0 == p.vehicle).map(|v| v.1) else {
        return;
    };
    let (w, h) = (1400.0, 420.0);
    let x_lo = end[0] - 260.0;
    let x_hi = end[0] + 60.0;
    let kx = w / (x_hi - x_lo);
    let ky = 8.0 * kx;
    let px = |x: f64| (x - x_lo) * kx;
    let pz = |z: f64| h / 2.0 + z * ky;
    let mut s = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}'>\n<rect width='100%' height='100%' fill='#eef0e8'/>\n"
    );
    // The road on the bottom straight (centreline z = 0).
    let half = if end[0] < 1600.0 { 2.5 } else { 3.25 };
    s += &format!(
        "<rect x='0' y='{:.1}' width='{w}' height='{:.1}' fill='#9a9a9a'/>\n",
        pz(-half),
        2.0 * half * ky
    );
    s += &format!(
        "<line x1='0' x2='{w}' y1='{0:.1}' y2='{0:.1}' stroke='white' stroke-width='2' stroke-dasharray='20 14'/>\n",
        pz(0.0)
    );
    for (k, (_, vs, rs)) in hist.iter().enumerate() {
        if k % 25 != 0 {
            continue;
        }
        let shade = 0.25 + 0.75 * k as f64 / hist.len() as f64;
        for v in vs {
            if v.1[0] < x_lo || v.1[0] > x_hi || v.1[1].abs() > 6.0 {
                continue;
            }
            let c = if v.0 == p.vehicle {
                "#1d3557"
            } else {
                "#6c757d"
            };
            s += &format!(
                "<circle cx='{:.1}' cy='{:.1}' r='4' fill='{c}' fill-opacity='{shade:.2}'/>\n",
                px(v.1[0]),
                pz(v.1[1])
            );
        }
        for r in rs {
            if r[0] < x_lo || r[0] > x_hi || r[1].abs() > 6.0 {
                continue;
            }
            s += &format!(
                "<circle cx='{:.1}' cy='{:.1}' r='3' fill='#e76f51' fill-opacity='{shade:.2}'/>\n",
                px(r[0]),
                pz(r[1])
            );
        }
    }
    s += &format!(
        "<text x='16' y='26' font-family='sans-serif' font-size='17'>Traffic T4: one overtake from above, every 0.5 s over the last 24 s (lateral ×8; vehicles at their front bumper). Blue: the passing {}, red: riders, grey: other vehicles.</text>\n",
        p.class.name()
    );
    s += &format!(
        "<text x='16' y='{}' font-family='sans-serif' font-size='14'>closest gap {:.2} m (legal {:.1} m), {:.1} m/s faster than the rider{}</text>\n</svg>\n",
        h - 16.0,
        p.clearance_m,
        p.legal_m,
        p.rel_speed,
        if p.lunatic { ", a lunatic" } else { "" }
    );
    std::fs::write(out, s).expect("write svg");
}
