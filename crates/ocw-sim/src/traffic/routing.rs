//! Routes on the road graph (spec 18B): a seeded walk for through traffic
//! (it keeps to the bigger and straighter road, never against a one-way
//! road or onto an edge its class may not use) and a shortest route by
//! time for vehicles with a destination (buses, closures of T13).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use ocw_world::RoadNetwork;
use ocw_world::graph::VertexId;
use ocw_world::network::splitmix;

use super::Bubble;
use super::kin::{STRAIGHT_R, turn_radius};
use super::plan::Step;
use super::rules::{EdgeRule, VehicleClass};

/// A uniform number in [0, 1) from a hash.
pub fn unit_f(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// A hash of several values under a seed (order-independent of the
/// simulation's iteration: each random choice has its own key).
pub fn key(seed: u64, a: u64, b: u64, c: u64) -> u64 {
    splitmix(splitmix(splitmix(seed ^ a) ^ b.rotate_left(21)) ^ c.rotate_left(42))
}

/// The edges a vehicle may leave `v` on, with their direction (not back
/// along `arrived`).
pub fn exits(
    net: &RoadNetwork,
    rules: &[EdgeRule],
    class: VehicleClass,
    v: VertexId,
    arrived: Option<usize>,
) -> Vec<(usize, bool)> {
    let mut out = Vec::new();
    for &e in net.edges_at(v) {
        if Some(e) == arrived {
            continue;
        }
        let edge = net.edge(e);
        // A loop edge can be left both ways.
        for forward in [true, false] {
            let start = if forward { edge.from } else { edge.to };
            if start == v && rules[e].allows(class, forward) {
                out.push((e, forward));
            }
        }
    }
    out
}

/// Whether a vertex is where traffic leaves the graph: the region's
/// boundary (the road goes on outside) or a dead end.
pub fn terminal(net: &RoadNetwork, v: VertexId) -> bool {
    net.graph().boundary_vertices.contains(&v) || net.edges_at(v).len() <= 1
}

/// Whether `p` is inside any bubble enlarged by `margin`.
pub fn inside(bubbles: &[Bubble], p: [f64; 2], margin: f64) -> bool {
    bubbles.iter().any(|b| {
        let (dx, dz) = (p[0] - b.center[0], p[1] - b.center[1]);
        let r = b.radius + margin;
        dx * dx + dz * dz <= r * r
    })
}

/// A step of `edge` for `class` at the driver's `factor`.
pub fn make_step(
    rules: &[EdgeRule],
    class: VehicleClass,
    edge: usize,
    forward: bool,
    factor: f64,
) -> Step {
    let limit = rules[edge].speed.min(class.traits().top_speed);
    Step {
        edge: edge as u32,
        forward,
        speed: (limit * factor).max(1.0) as f32,
        turn_r: STRAIGHT_R as f32,
    }
}

/// Sets the turn radius of the last step of `route` into `next`.
pub fn link_turn(net: &RoadNetwork, route: &mut [Step], next: &Step) {
    if let Some(last) = route.last_mut() {
        let arrive = net.arriving_direction(last.edge as usize, last.forward, 20.0);
        let leave = net.leaving_direction(next.edge as usize, next.forward, 20.0);
        last.turn_r = if last.edge == next.edge && last.forward != next.forward {
            5.0
        } else {
            turn_radius(arrive, leave) as f32
        };
    }
}

/// Where a walk stops.
#[derive(Debug, Clone, Copy)]
pub struct WalkLimits<'a> {
    /// Stop at a vertex outside all of these (enlarged by `margin`).
    pub bubbles: &'a [Bubble],
    pub margin: f64,
    /// Stop after this many metres.
    pub max_m: f64,
    pub max_steps: usize,
}

/// Continues a route from the end of `last` with a seeded walk: at each
/// vertex the allowed exits are weighted by the road's traffic and how
/// straight they go on. Stops at a terminal vertex, outside the bubbles,
/// or at the limits. Returns the new steps (the turn radius of `last` is
/// not set; see [`link_turn`]).
pub fn walk(
    net: &RoadNetwork,
    rules: &[EdgeRule],
    class: VehicleClass,
    last: (usize, bool),
    factor: f64,
    seed: u64,
    lim: &WalkLimits,
) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::new();
    let (mut e, mut fwd) = last;
    let mut length = 0.0;
    for n in 0..lim.max_steps {
        let v = net.end_vertex(e, fwd);
        if terminal(net, v) {
            break;
        }
        let p = net.graph().vertices[v.0 as usize];
        if !inside(lim.bubbles, p, lim.margin) || length >= lim.max_m {
            break;
        }
        let mut ex = exits(net, rules, class, v, Some(e));
        if ex.is_empty() {
            // Nowhere to go on: turn round where the road allows it.
            if rules[e].allows(class, !fwd) {
                ex.push((e, !fwd));
            } else {
                break;
            }
        }
        let arrive = net.arriving_direction(e, fwd, 20.0);
        let weights: Vec<f64> = ex
            .iter()
            .map(|&(x, xf)| {
                let d = net.leaving_direction(x, xf, 20.0);
                let straight = (1.0 + arrive[0] * d[0] + arrive[1] * d[1]) * 0.5;
                (rules[x].aadt + 20.0).sqrt() * (straight * straight + 0.04)
            })
            .collect();
        let total: f64 = weights.iter().sum();
        let mut u = unit_f(key(seed, n as u64, e as u64, 0x77)) * total;
        let mut pick = ex.len() - 1;
        for (i, w) in weights.iter().enumerate() {
            if u < *w {
                pick = i;
                break;
            }
            u -= w;
        }
        (e, fwd) = ex[pick];
        let st = make_step(rules, class, e, fwd, factor);
        link_turn(net, &mut out, &st);
        out.push(st);
        length += net.geometry(e).length();
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Node {
    cost: f64,
    state: usize,
}

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| o.state.cmp(&self.state))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The quickest route by the speed limits from the start of `from` (edge,
/// forward) to vertex `to`, only over edges `class` may use in their
/// direction: the steps (edge, forward), `from` included. `None` when no
/// legal route exists.
pub fn shortest_route(
    net: &RoadNetwork,
    rules: &[EdgeRule],
    class: VehicleClass,
    from: (usize, bool),
    to: VertexId,
) -> Option<Vec<(usize, bool)>> {
    if !rules[from.0].allows(class, from.1) {
        return None;
    }
    let state = |e: usize, f: bool| e * 2 + usize::from(!f);
    let n = net.graph().edges.len() * 2;
    let mut cost = vec![f64::INFINITY; n];
    let mut prev = vec![usize::MAX; n];
    let mut heap = BinaryHeap::new();
    let time =
        |e: usize| net.edge(e).length_m() / rules[e].speed.min(class.traits().top_speed).max(1.0);
    let s0 = state(from.0, from.1);
    cost[s0] = time(from.0);
    heap.push(Node {
        cost: cost[s0],
        state: s0,
    });
    while let Some(Node { cost: c, state: s }) = heap.pop() {
        if c > cost[s] {
            continue;
        }
        let (e, f) = (s / 2, s.is_multiple_of(2));
        let v = net.end_vertex(e, f);
        if v == to {
            let mut out = vec![(e, f)];
            let mut k = s;
            while prev[k] != usize::MAX {
                k = prev[k];
                out.push((k / 2, k.is_multiple_of(2)));
            }
            out.reverse();
            return Some(out);
        }
        for (x, xf) in exits(net, rules, class, v, Some(e)) {
            let t = state(x, xf);
            let nc = c + time(x);
            if nc < cost[t] {
                cost[t] = nc;
                prev[t] = s;
                heap.push(Node { cost: nc, state: t });
            }
        }
    }
    None
}
