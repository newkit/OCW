//! Straight skeleton roofs (spec §14 Phase 8 T4, v3.26): hipped and gabled
//! roofs over any simple footprint – L-, T- and U-shapes, odd corners and
//! courtyards.
//!
//! The eaves of every wall move inwards at the same speed (the wavefront);
//! where they meet they trace the hips, valleys and ridges of a roof of one
//! pitch – the straight skeleton of the footprint. Each wall gets one roof
//! face: a polygon over its eave edge, the height of a point being the slope
//! times its offset (the time the wavefront reached it). A gabled roof turns
//! the triangular hip ends into vertical gables: their apex moves along its
//! ridge out to the wall ([`Skeleton::make_gables`]).
//!
//! The wavefront is simulated event by event – edge events (an edge shrinks
//! to nothing) and split events (a reflex corner runs into an edge, which
//! also joins a courtyard's wavefront to the outer one) – with all events
//! recomputed after each: O(n³), fine for footprints of up to
//! [`MAX_POINTS`] points. Edges whose wavefronts coincide (parallel walls
//! facing each other: the ridge of a rectangular arm) collapse at once.
//! Pure and deterministic; anything degenerate gives `None`, and the caller
//! draws a flat roof.
//!
//! Rings follow the building convention ([`crate::buildings`]): the outer
//! ring counter-clockwise in (x, z), holes clockwise, open; the building is
//! on the left of every edge.

/// Footprints with more points (all rings) get no skeleton.
pub const MAX_POINTS: usize = 64;

/// A node of the skeleton: its place in plan (x, z) and its offset – the
/// horizontal distance from the eaves, so its height above them is
/// slope × `t`. Ring points have `t = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkNode {
    pub p: [f64; 2],
    pub t: f64,
}

/// The straight skeleton of a footprint as roof faces.
#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    /// The ring points first (outer ring, then the holes, in order), then
    /// the inner nodes.
    pub nodes: Vec<SkNode>,
    /// Per ring edge (the outer ring's edges first, then each hole's; edge
    /// `i` of a ring runs from its point `i` to `i + 1`) the face over it:
    /// node indices counter-clockwise in plan, starting with the edge's two
    /// points.
    pub faces: Vec<Vec<usize>>,
    /// Faces that are vertical gables ([`Skeleton::make_gables`]).
    pub gable: Vec<bool>,
    /// The largest offset of a node (half the width of the widest part).
    pub max_offset: f64,
    /// Per edge: the edge before it in its ring and the one after it.
    pub neighbours: Vec<(usize, usize)>,
}

type P = [f64; 2];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}

fn mul(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn turn(a: P, b: P, c: P) -> f64 {
    cross(sub(b, a), sub(c, a))
}

fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Twice the signed area of a polygon (positive: counter-clockwise).
fn area2(pts: &[P]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| cross(pts[i], pts[(i + 1) % n])).sum()
}

/// An edge's line: a point on it, its direction and its inward normal
/// (unit vectors; the building is on the left).
#[derive(Debug, Clone, Copy)]
struct Line {
    a: P,
    d: P,
    n: P,
}

/// A vertex of the wavefront.
#[derive(Debug, Clone, Copy)]
struct Wave {
    /// Position at time `t0`, velocity.
    p: P,
    t0: f64,
    v: P,
    /// The node it started at.
    node: usize,
    /// The edge before it (into it) and after it.
    el: usize,
    er: usize,
    prev: usize,
    next: usize,
    alive: bool,
    /// Its two edges face each other on one line (no velocity): it
    /// collapses at once.
    fast: bool,
    reflex: bool,
}

enum Event {
    Edge(usize),
    Split { r: usize, a: usize },
}

struct Sim {
    lines: Vec<Line>,
    nodes: Vec<SkNode>,
    w: Vec<Wave>,
    /// Per edge: the segments (node pairs) bounding its face.
    segs: Vec<Vec<(usize, usize)>>,
    now: f64,
    eps: f64,
}

impl Sim {
    fn pos(&self, i: usize, t: f64) -> P {
        let w = &self.w[i];
        add(w.p, mul(w.v, t - w.t0))
    }

    fn node(&mut self, p: P) -> usize {
        self.nodes.push(SkNode { p, t: self.now });
        self.nodes.len() - 1
    }

    /// The wavefront vertex between edges `el` and `er` moves so that it
    /// stays on both offset lines: `v · n1 = v · n2 = 1`.
    fn velocity(&self, el: usize, er: usize) -> Option<P> {
        let (n1, n2) = (self.lines[el].n, self.lines[er].n);
        let c = 1.0 + dot(n1, n2);
        (c > 1e-9).then(|| mul(add(n1, n2), 1.0 / c))
    }

    fn vertex(&mut self, p: P, node: usize, el: usize, er: usize) -> usize {
        let v = self.velocity(el, er);
        let reflex = cross(self.lines[el].d, self.lines[er].d) < -1e-12;
        self.w.push(Wave {
            p,
            t0: self.now,
            v: v.unwrap_or([0.0, 0.0]),
            node,
            el,
            er,
            prev: usize::MAX,
            next: usize::MAX,
            alive: true,
            fast: v.is_none(),
            reflex,
        });
        self.w.len() - 1
    }

    fn link(&mut self, a: usize, b: usize) {
        self.w[a].next = b;
        self.w[b].prev = a;
    }

    /// The vertex dies at node `to`: its arc separates its two faces.
    fn die(&mut self, i: usize, to: usize) {
        let w = self.w[i];
        self.w[i].alive = false;
        self.seg(w.node, to, w.el, w.er);
    }

    fn seg(&mut self, a: usize, b: usize, e1: usize, e2: usize) {
        if a != b {
            self.segs[e1].push((a, b));
            if e2 != e1 {
                self.segs[e2].push((a, b));
            }
        }
    }

    fn loop_len(&self, i: usize) -> usize {
        let mut n = 1;
        let mut j = self.w[i].next;
        while j != i && n < 1_000_000 {
            n += 1;
            j = self.w[j].next;
        }
        n
    }

    /// Closes a wavefront of two vertices (its two edges lie on one line).
    fn close2(&mut self, u: usize, w: usize) {
        let (pu, pw) = (self.pos(u, self.now), self.pos(w, self.now));
        let nu = self.node(pu);
        let nw = if dist(pu, pw) <= self.eps {
            nu
        } else {
            self.node(pw)
        };
        let (eu, ew) = (self.w[u].er, self.w[w].er);
        self.die(u, nu);
        self.die(w, nw);
        self.seg(nu, nw, eu, ew);
    }

    /// A vertex whose edges face each other on one line: the zero-width
    /// strip between it and the nearer neighbour on that line collapses
    /// (a ridge), and the vertex moves there.
    fn collapse_fast(&mut self, x: usize) {
        let (p, q) = (self.w[x].prev, self.w[x].next);
        if p == q {
            self.close2(x, q);
            return;
        }
        let now = self.now;
        let px = self.pos(x, now);
        let (pp, pq) = (self.pos(p, now), self.pos(q, now));
        let (el, er) = (self.w[x].el, self.w[x].er);
        if dist(px, pq) <= dist(px, pp) {
            let ny = self.node(pq);
            self.die(x, ny);
            self.die(q, ny);
            let qr = self.w[q].er;
            let qn = self.w[q].next;
            let y = self.vertex(pq, ny, el, qr);
            self.link(p, y);
            self.link(y, qn);
        } else {
            let ny = self.node(pp);
            self.die(x, ny);
            self.die(p, ny);
            let pl = self.w[p].el;
            let pprev = self.w[p].prev;
            let y = self.vertex(pp, ny, pl, er);
            self.link(pprev, y);
            self.link(y, q);
        }
    }

    fn edge_event(&mut self, u: usize) {
        let w = self.w[u].next;
        let now = self.now;
        let at = mul(add(self.pos(u, now), self.pos(w, now)), 0.5);
        let n = self.node(at);
        if self.loop_len(u) == 3 {
            let v = self.w[w].next;
            for i in [u, w, v] {
                self.die(i, n);
            }
            return;
        }
        let (prev, next) = (self.w[u].prev, self.w[w].next);
        let (el, er) = (self.w[u].el, self.w[w].er);
        self.die(u, n);
        self.die(w, n);
        let x = self.vertex(at, n, el, er);
        self.link(prev, x);
        self.link(x, next);
    }

    fn split_event(&mut self, r: usize, a: usize) {
        let now = self.now;
        let at = self.pos(r, now);
        let n = self.node(at);
        let b = self.w[a].next;
        let e = self.w[a].er;
        let (rp, rn) = (self.w[r].prev, self.w[r].next);
        let (rl, rr) = (self.w[r].el, self.w[r].er);
        self.die(r, n);
        let x1 = self.vertex(at, n, e, rr);
        let x2 = self.vertex(at, n, rl, e);
        self.link(a, x1);
        self.link(x1, rn);
        self.link(rp, x2);
        self.link(x2, b);
    }

    /// The earliest event after `now` (edge events first on a tie).
    fn next_event(&self) -> Option<(f64, Event)> {
        let now = self.now;
        let eps = self.eps;
        let mut best: Option<(f64, Event)> = None;
        let take = |t: f64, ev: Event, best: &mut Option<(f64, Event)>| {
            let better = match best {
                None => true,
                Some((bt, bev)) => {
                    t < *bt - 1e-9
                        || (t <= *bt + 1e-9
                            && matches!(ev, Event::Edge(_))
                            && matches!(bev, Event::Split { .. }))
                }
            };
            if better {
                *best = Some((t, ev));
            }
        };
        for u in 0..self.w.len() {
            let wu = &self.w[u];
            if !wu.alive {
                continue;
            }
            let w = wu.next;
            if w == u {
                continue;
            }
            let d = self.lines[wu.er].d;
            let len = dot(sub(self.pos(w, now), self.pos(u, now)), d);
            let rate = dot(sub(self.w[w].v, wu.v), d);
            let t = if len <= eps && rate <= 1e-9 {
                now
            } else if rate < -1e-12 {
                now + len.max(0.0) / -rate
            } else {
                continue;
            };
            take(t, Event::Edge(u), &mut best);
        }
        for r in 0..self.w.len() {
            let wr = self.w[r];
            if !wr.alive || !wr.reflex || wr.fast {
                continue;
            }
            let pr = self.pos(r, now);
            for a in 0..self.w.len() {
                let wa = &self.w[a];
                if !wa.alive || a == r || wa.next == r {
                    continue;
                }
                let b = wa.next;
                let e = wa.er;
                if e == wr.el || e == wr.er {
                    continue;
                }
                let l = self.lines[e];
                let f = dot(sub(pr, l.a), l.n) - now;
                let slope = dot(wr.v, l.n) - 1.0;
                if f < -eps || slope >= -1e-12 {
                    continue;
                }
                let t = now + f.max(0.0) / -slope;
                if let Some((bt, _)) = &best
                    && t > *bt + 1e-9
                {
                    continue;
                }
                let p = self.pos(r, t);
                let (pa, pb) = (self.pos(a, t), self.pos(b, t));
                let len = dot(sub(pb, pa), l.d);
                let s = dot(sub(p, pa), l.d);
                if s < -eps || s > len + eps {
                    continue;
                }
                take(t, Event::Split { r, a }, &mut best);
            }
        }
        best
    }

    fn run(&mut self, max_steps: usize) -> Option<()> {
        for _ in 0..max_steps {
            let alive: Vec<usize> = (0..self.w.len()).filter(|&i| self.w[i].alive).collect();
            if alive.is_empty() {
                return Some(());
            }
            if let Some(&x) = alive.iter().find(|&&i| self.w[i].fast) {
                self.collapse_fast(x);
                continue;
            }
            if let Some(&u) = alive.iter().find(|&&i| self.loop_len(i) <= 2) {
                let w = self.w[u].next;
                if w == u {
                    let p = self.pos(u, self.now);
                    let n = self.node(p);
                    self.die(u, n);
                } else {
                    self.close2(u, w);
                }
                continue;
            }
            let (t, ev) = self.next_event()?;
            if !t.is_finite() || t < self.now - self.eps {
                return None;
            }
            self.now = t.max(self.now);
            match ev {
                Event::Edge(u) => self.edge_event(u),
                Event::Split { r, a } => self.split_event(r, a),
            }
        }
        None
    }
}

/// Ear clipping of a simple counter-clockwise polygon that keeps collinear
/// points (a skeleton face's ridge may pass through a node of its
/// neighbour: dropping it would leave a crack). Indices into `pts`.
pub fn triangulate_face(pts: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let scale = pts
        .iter()
        .flat_map(|p| p.iter().map(|v| v.abs()))
        .fold(1.0, f64::max);
    let eps = 1e-12 * scale * scale;
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut tris = Vec::new();
    let inside = |p: P, a: P, b: P, c: P| {
        turn(a, b, p) >= -eps && turn(b, c, p) >= -eps && turn(c, a, p) >= -eps
    };
    let mut guard = 0;
    while idx.len() > 3 && guard < pts.len() * pts.len() + 16 {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if turn(a, b, c) <= eps {
                continue;
            }
            let blocked = idx.iter().any(|&j| {
                let p = pts[j];
                j != ia && j != ib && j != ic && p != a && p != b && p != c && inside(p, a, b, c)
            });
            if !blocked {
                tris.push([ia, ib, ic]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            // Only flat or reflex corners left: drop a flat one.
            let flat = (0..m).find(|&i| {
                let (a, b, c) = (
                    pts[idx[(i + m - 1) % m]],
                    pts[idx[i]],
                    pts[idx[(i + 1) % m]],
                );
                turn(a, b, c).abs() <= eps
            });
            match flat {
                Some(i) => {
                    idx.remove(i);
                }
                None => break,
            }
        }
    }
    if idx.len() == 3 && turn(pts[idx[0]], pts[idx[1]], pts[idx[2]]) > eps {
        tris.push([idx[0], idx[1], idx[2]]);
    }
    tris
}

/// The straight skeleton of a footprint (outer ring counter-clockwise,
/// holes clockwise, open rings). `None` for degenerate footprints, more
/// than [`MAX_POINTS`] points, or when the faces do not tile the footprint.
pub fn skeleton(outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>]) -> Option<Skeleton> {
    let rings: Vec<&[[f64; 2]]> = std::iter::once(outer)
        .chain(holes.iter().map(Vec::as_slice))
        .collect();
    let total: usize = rings.iter().map(|r| r.len()).sum();
    if outer.len() < 3 || total > MAX_POINTS || rings.iter().any(|r| r.len() < 3) {
        return None;
    }
    if area2(outer) <= 0.0 || holes.iter().any(|h| area2(h) >= 0.0) {
        return None;
    }
    let scale = rings
        .iter()
        .flat_map(|r| r.iter())
        .flat_map(|p| p.iter().map(|v| v.abs()))
        .fold(1.0, f64::max);
    let eps = 1e-9 * scale;
    let mut sim = Sim {
        lines: Vec::with_capacity(total),
        nodes: Vec::with_capacity(total * 4),
        w: Vec::with_capacity(total * 4),
        segs: vec![Vec::new(); total],
        now: 0.0,
        eps,
    };
    let mut neighbours = Vec::with_capacity(total);
    let mut start = 0;
    for r in &rings {
        let n = r.len();
        for i in 0..n {
            let (a, b) = (r[i], r[(i + 1) % n]);
            let len = dist(a, b);
            if len <= eps * 10.0 {
                return None; // repeated points
            }
            let d = mul(sub(b, a), 1.0 / len);
            sim.lines.push(Line {
                a,
                d,
                n: [-d[1], d[0]],
            });
            neighbours.push((start + (i + n - 1) % n, start + (i + 1) % n));
        }
        start += n;
    }
    let mut start = 0;
    for r in &rings {
        let n = r.len();
        for (i, p) in r.iter().enumerate() {
            sim.nodes.push(SkNode { p: *p, t: 0.0 });
            let el = start + (i + n - 1) % n;
            sim.vertex(*p, start + i, el, start + i);
        }
        for i in 0..n {
            let (a, b) = (start + i, start + (i + 1) % n);
            sim.link(a, b);
        }
        start += n;
    }
    sim.run(40 * total + 200)?;
    // Weld nodes at the same place (simultaneous events).
    let weld = 1e-6 * scale;
    let mut rep: Vec<usize> = (0..sim.nodes.len()).collect();
    for i in total..sim.nodes.len() {
        if let Some(j) =
            (0..i).find(|&j| rep[j] == j && dist(sim.nodes[i].p, sim.nodes[j].p) <= weld)
        {
            rep[i] = j;
        }
    }
    let mut faces = Vec::with_capacity(total);
    let mut start = 0;
    for r in &rings {
        let n = r.len();
        for i in 0..n {
            let e = start + i;
            let (na, nb) = (start + i, start + (i + 1) % n);
            let mut segs: Vec<(usize, usize)> = sim.segs[e]
                .iter()
                .map(|&(a, b)| (rep[a].min(rep[b]), rep[a].max(rep[b])))
                .filter(|(a, b)| a != b)
                .collect();
            segs.sort_unstable();
            segs.dedup();
            let mut used = vec![false; segs.len()];
            let mut face = vec![na, nb];
            let mut cur = nb;
            while cur != na {
                let k = (0..segs.len())
                    .find(|&k| !used[k] && (segs[k].0 == cur || segs[k].1 == cur))?;
                used[k] = true;
                cur = if segs[k].0 == cur {
                    segs[k].1
                } else {
                    segs[k].0
                };
                if cur != na {
                    face.push(cur);
                }
            }
            faces.push(face);
        }
        start += n;
    }
    // Only the nodes the faces use (ring points stay first).
    let mut keep = vec![usize::MAX; sim.nodes.len()];
    let mut nodes: Vec<SkNode> = sim.nodes[..total].to_vec();
    keep[..total]
        .iter_mut()
        .enumerate()
        .for_each(|(i, k)| *k = i);
    for f in &mut faces {
        for i in f.iter_mut() {
            if keep[*i] == usize::MAX {
                keep[*i] = nodes.len();
                nodes.push(sim.nodes[*i]);
            }
            *i = keep[*i];
        }
    }
    sim.nodes = nodes;
    // The faces must tile the footprint.
    let want: f64 = rings.iter().map(|r| area2(r)).sum();
    let mut got = 0.0;
    for f in &faces {
        let pts: Vec<P> = f.iter().map(|&i| sim.nodes[i].p).collect();
        let a = area2(&pts);
        if a < -1e-6 * scale * scale {
            return None;
        }
        got += a;
    }
    if (got - want).abs() > 1e-4 * want.abs() + 1e-6 * scale * scale {
        return None;
    }
    let max_offset = sim.nodes.iter().map(|n| n.t).fold(0.0, f64::max);
    if !max_offset.is_finite() || sim.nodes.iter().any(|n| n.t < -eps) {
        return None;
    }
    Some(Skeleton {
        nodes: sim.nodes,
        gable: vec![false; faces.len()],
        faces,
        max_offset,
        neighbours,
    })
}

impl Skeleton {
    /// Turns the hip ends into gables: every face that is a triangle over
    /// its edge whose apex ends one ridge (between the faces of the edges
    /// before and after it) becomes a vertical gable, its apex moved along
    /// the ridge out to the edge (the neighbouring faces stay planar). The
    /// number of gables made.
    pub fn make_gables(&mut self) -> usize {
        let mut made = 0;
        for e in 0..self.faces.len() {
            if self.faces[e].len() != 3 {
                continue;
            }
            let (na, nb, apex) = (self.faces[e][0], self.faces[e][1], self.faces[e][2]);
            if self.nodes[apex].t <= 0.0 {
                continue;
            }
            let (ep, en) = self.neighbours[e];
            if self.gable[ep] || self.gable[en] || ep == en {
                continue;
            }
            let (fp, fnx) = (&self.faces[ep], &self.faces[en]);
            // The face before: [.., na, apex, ridge, ..]; the face after
            // ends with [.., ridge, apex].
            if fp.len() < 4 || fp[1] != na || fp[2] != apex || fnx.len() < 4 {
                continue;
            }
            let ridge = fp[3];
            if fnx[fnx.len() - 1] != apex || fnx[fnx.len() - 2] != ridge || fnx[0] != nb {
                continue;
            }
            let (pa, pb) = (self.nodes[na].p, self.nodes[nb].p);
            let len = dist(pa, pb);
            if len <= 1e-6 {
                continue;
            }
            let d = mul(sub(pb, pa), 1.0 / len);
            let n_in = [-d[1], d[0]];
            let (pn, pm) = (self.nodes[apex].p, self.nodes[ridge].p);
            let dir = sub(pn, pm);
            let toward = dot(dir, n_in);
            if toward >= -1e-9 {
                continue; // the ridge does not run towards this edge
            }
            let off = dot(sub(pn, pa), n_in);
            let moved = add(pn, mul(dir, off / -toward));
            let s = dot(sub(moved, pa), d);
            if s < 0.02 * len || s > 0.98 * len {
                continue;
            }
            // Its height: its offset from the edge before (= the one after).
            let pp = self.nodes[fp[0]].p;
            let lp = dist(pp, pa).max(1e-9);
            let dp = mul(sub(pa, pp), 1.0 / lp);
            let t = dot(sub(moved, pp), [-dp[1], dp[0]]);
            if t <= 0.0 {
                continue;
            }
            self.nodes[apex] = SkNode { p: moved, t };
            self.gable[e] = true;
            made += 1;
        }
        if made > 0 {
            self.max_offset = self.nodes.iter().map(|n| n.t).fold(0.0, f64::max);
        }
        made
    }

    /// The points of face `e` with their offsets, (x, z) and t.
    pub fn face_nodes(&self, e: usize) -> Vec<SkNode> {
        self.faces[e].iter().map(|&i| self.nodes[i]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(pts: &[P]) -> f64 {
        area2(pts) / 2.0
    }

    fn face_area(s: &Skeleton, e: usize) -> f64 {
        let pts: Vec<P> = s.faces[e].iter().map(|&i| s.nodes[i].p).collect();
        area(&pts)
    }

    /// Every face starts with its edge, winds counter-clockwise, the faces
    /// tile the footprint, every inner edge is shared by two faces in
    /// opposite directions, and the offset of each node is its distance
    /// from its face's eave line.
    fn check(s: &Skeleton, outer: &[P], holes: &[Vec<P>]) {
        let want = area(outer) + holes.iter().map(|h| area(h)).sum::<f64>();
        let got: f64 = (0..s.faces.len()).map(|e| face_area(s, e)).sum();
        assert!((got - want).abs() < 1e-6 * want, "{got} vs {want}");
        let mut edges = std::collections::HashMap::new();
        for (e, f) in s.faces.iter().enumerate() {
            assert!(face_area(s, e) > -1e-9, "face {e}");
            let (a, b) = (s.nodes[f[0]].p, s.nodes[f[1]].p);
            let d = mul(sub(b, a), 1.0 / dist(a, b));
            for &i in f {
                let off = dot(sub(s.nodes[i].p, a), [-d[1], d[0]]);
                assert!(
                    (off - s.nodes[i].t).abs() < 1e-6,
                    "face {e} node {i}: {off} vs {}",
                    s.nodes[i].t
                );
            }
            for k in 0..f.len() {
                *edges.entry((f[k], f[(k + 1) % f.len()])).or_insert(0) += 1;
            }
        }
        let n_ring = outer.len() + holes.iter().map(Vec::len).sum::<usize>();
        for (&(a, b), &n) in &edges {
            assert_eq!(n, 1, "edge {a} → {b} twice");
            if a < n_ring && b < n_ring {
                continue; // an eave
            }
            assert_eq!(edges.get(&(b, a)), Some(&1), "edge {a} → {b} not shared");
        }
    }

    fn rect(w: f64, d: f64) -> Vec<P> {
        vec![[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]]
    }

    fn ridge_nodes(s: &Skeleton) -> Vec<P> {
        let top = s.max_offset;
        let mut v: Vec<P> = s
            .nodes
            .iter()
            .filter(|n| (n.t - top).abs() < 1e-6)
            .map(|n| n.p)
            .collect();
        v.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        v.dedup_by(|a, b| dist(*a, *b) < 1e-6);
        v
    }

    #[test]
    fn rectangle_gives_the_hipped_roof() {
        let r = rect(12.0, 8.0);
        let s = skeleton(&r, &[]).unwrap();
        check(&s, &r, &[]);
        assert!((s.max_offset - 4.0).abs() < 1e-9);
        assert_eq!(ridge_nodes(&s), vec![[4.0, 4.0], [8.0, 4.0]]);
        assert_eq!(
            s.faces.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![4, 3, 4, 3]
        );
        // A square: a pyramid.
        let sq = rect(10.0, 10.0);
        let s = skeleton(&sq, &[]).unwrap();
        check(&s, &sq, &[]);
        assert_eq!(ridge_nodes(&s), vec![[5.0, 5.0]]);
    }

    fn l_shape() -> Vec<P> {
        vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 8.0],
            [8.0, 8.0],
            [8.0, 20.0],
            [0.0, 20.0],
        ]
    }

    #[test]
    fn l_t_and_u_shapes_have_their_ridges() {
        let l = l_shape();
        let s = skeleton(&l, &[]).unwrap();
        check(&s, &l, &[]);
        assert!((s.max_offset - 4.0).abs() < 1e-9);
        // The two ridges along the arms' middles, meeting at (4, 4).
        assert_eq!(ridge_nodes(&s), vec![[4.0, 4.0], [4.0, 16.0], [16.0, 4.0]]);
        // A T: a 24 m bar 8 m deep with an 8 m wide stem of 10 m.
        let t = vec![
            [0.0, 0.0],
            [24.0, 0.0],
            [24.0, 8.0],
            [16.0, 8.0],
            [16.0, 18.0],
            [8.0, 18.0],
            [8.0, 8.0],
            [0.0, 8.0],
        ];
        let s = skeleton(&t, &[]).unwrap();
        check(&s, &t, &[]);
        assert_eq!(
            ridge_nodes(&s),
            vec![[4.0, 4.0], [12.0, 4.0], [12.0, 14.0], [20.0, 4.0]]
        );
        // A U: two 6 m wide arms on a 6 m deep base.
        let u = vec![
            [0.0, 0.0],
            [18.0, 0.0],
            [18.0, 16.0],
            [12.0, 16.0],
            [12.0, 6.0],
            [6.0, 6.0],
            [6.0, 16.0],
            [0.0, 16.0],
        ];
        let s = skeleton(&u, &[]).unwrap();
        check(&s, &u, &[]);
        assert!((s.max_offset - 3.0).abs() < 1e-9);
        assert_eq!(
            ridge_nodes(&s),
            vec![[3.0, 3.0], [3.0, 13.0], [15.0, 3.0], [15.0, 13.0]]
        );
        // Unequal arms (10 m and 6 m wide): the ridges at different heights.
        let l2 = vec![
            [0.0, 0.0],
            [24.0, 0.0],
            [24.0, 10.0],
            [6.0, 10.0],
            [6.0, 20.0],
            [0.0, 20.0],
        ];
        let s = skeleton(&l2, &[]).unwrap();
        check(&s, &l2, &[]);
        assert!((s.max_offset - 5.0).abs() < 1e-9);
    }

    #[test]
    fn convex_and_concave_polygons() {
        // A regular hexagon: one apex at the centre.
        let hex: Vec<P> = (0..6)
            .map(|i| {
                let a = i as f64 * std::f64::consts::FRAC_PI_3;
                [5.0 * a.cos(), 5.0 * a.sin()]
            })
            .collect();
        let s = skeleton(&hex, &[]).unwrap();
        check(&s, &hex, &[]);
        assert!((s.max_offset - 5.0 * (std::f64::consts::PI / 6.0).cos()).abs() < 1e-6);
        assert_eq!(ridge_nodes(&s).len(), 1);
        assert!(dist(ridge_nodes(&s)[0], [0.0, 0.0]) < 1e-6);
        // An irregular pentagon and a concave arrow, a zigzag terrace.
        let penta = vec![
            [0.0, 0.0],
            [14.0, 1.0],
            [16.0, 9.0],
            [7.0, 13.0],
            [-2.0, 7.0],
        ];
        let s = skeleton(&penta, &[]).unwrap();
        check(&s, &penta, &[]);
        let arrow = vec![[0.0, 0.0], [10.0, 5.0], [0.0, 10.0], [3.0, 5.0]];
        let s = skeleton(&arrow, &[]).unwrap();
        check(&s, &arrow, &[]);
        let zig = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 3.0],
            [20.0, 3.0],
            [20.0, 11.0],
            [10.0, 11.0],
            [10.0, 8.0],
            [0.0, 8.0],
        ];
        let s = skeleton(&zig, &[]).unwrap();
        check(&s, &zig, &[]);
        // An oblique L (not axis-aligned).
        let (c, sn) = (0.6_f64, 0.8_f64);
        let rot: Vec<P> = l_shape()
            .iter()
            .map(|p| [c * p[0] - sn * p[1] + 100.0, sn * p[0] + c * p[1] - 50.0])
            .collect();
        let s = skeleton(&rot, &[]).unwrap();
        check(&s, &rot, &[]);
        assert!((s.max_offset - 4.0).abs() < 1e-6);
    }

    #[test]
    fn courtyards_get_their_own_faces() {
        let outer = rect(20.0, 20.0);
        let hole = vec![[6.0, 6.0], [6.0, 14.0], [14.0, 14.0], [14.0, 6.0]];
        let s = skeleton(&outer, std::slice::from_ref(&hole)).unwrap();
        check(&s, &outer, std::slice::from_ref(&hole));
        assert_eq!(s.faces.len(), 8);
        assert!((s.max_offset - 3.0).abs() < 1e-9);
        // A courtyard off the centre.
        let hole = vec![[3.0, 3.0], [3.0, 9.0], [9.0, 9.0], [9.0, 3.0]];
        let s = skeleton(&outer, std::slice::from_ref(&hole)).unwrap();
        check(&s, &outer, std::slice::from_ref(&hole));
    }

    #[test]
    fn gables_replace_the_hip_ends() {
        let r = rect(12.0, 8.0);
        let mut s = skeleton(&r, &[]).unwrap();
        assert_eq!(s.make_gables(), 2);
        assert_eq!(s.gable, vec![false, true, false, true]);
        assert_eq!(ridge_nodes(&s), vec![[0.0, 4.0], [12.0, 4.0]]);
        // The L: gables at both arm ends, the ridges reach the walls at
        // the same height.
        let l = l_shape();
        let mut s = skeleton(&l, &[]).unwrap();
        assert_eq!(s.make_gables(), 2);
        assert!(s.gable[1] && s.gable[4]);
        assert_eq!(ridge_nodes(&s), vec![[4.0, 4.0], [4.0, 20.0], [20.0, 4.0]]);
        assert!((s.max_offset - 4.0).abs() < 1e-9);
        // The faces next to a gable are still planar: offsets match.
        for e in [0, 2, 3, 5] {
            let f = &s.faces[e];
            let (a, b) = (s.nodes[f[0]].p, s.nodes[f[1]].p);
            let d = mul(sub(b, a), 1.0 / dist(a, b));
            for &i in f {
                let off = dot(sub(s.nodes[i].p, a), [-d[1], d[0]]);
                assert!((off - s.nodes[i].t).abs() < 1e-6);
            }
        }
        // A square pyramid has no ridge to turn into gables.
        let mut s = skeleton(&rect(10.0, 10.0), &[]).unwrap();
        assert_eq!(s.make_gables(), 0);
    }

    #[test]
    fn degenerate_footprints_give_none() {
        assert!(skeleton(&[[0.0, 0.0], [1.0, 0.0]], &[]).is_none());
        // Clockwise outer ring, zero area, repeated points.
        let mut cw = rect(4.0, 4.0);
        cw.reverse();
        assert!(skeleton(&cw, &[]).is_none());
        assert!(skeleton(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], &[]).is_none());
        assert!(skeleton(&[[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], &[]).is_none());
        // Too many points.
        let many: Vec<P> = (0..MAX_POINTS + 1)
            .map(|i| {
                let a = i as f64 / (MAX_POINTS + 1) as f64 * std::f64::consts::TAU;
                [10.0 * a.cos(), 10.0 * a.sin()]
            })
            .collect();
        assert!(skeleton(&many, &[]).is_none());
        // A self-intersecting bow tie.
        assert!(skeleton(&[[0.0, 0.0], [4.0, 4.0], [4.0, 0.0], [0.0, 4.0]], &[]).is_none());
    }

    #[test]
    fn faces_triangulate_with_their_collinear_points() {
        // A square with a point in the middle of its top edge.
        let p = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [1.0, 2.0], [0.0, 2.0]];
        let t = triangulate_face(&p);
        assert_eq!(t.len(), 3);
        assert!(t.iter().flatten().any(|&i| i == 3), "{t:?}");
        let a: f64 = t
            .iter()
            .map(|t| turn(p[t[0]], p[t[1]], p[t[2]]) / 2.0)
            .sum();
        assert!((a - 4.0).abs() < 1e-12);
        // Every T face of a real skeleton.
        let s = skeleton(&l_shape(), &[]).unwrap();
        for e in 0..s.faces.len() {
            let pts: Vec<P> = s.face_nodes(e).iter().map(|n| n.p).collect();
            let t = triangulate_face(&pts);
            let a: f64 = t
                .iter()
                .map(|t| turn(pts[t[0]], pts[t[1]], pts[t[2]]) / 2.0)
                .sum();
            assert!((a - area(&pts)).abs() < 1e-9);
        }
    }

    #[test]
    fn skeletons_are_deterministic() {
        let a = skeleton(&l_shape(), &[]).unwrap();
        let b = skeleton(&l_shape(), &[]).unwrap();
        assert_eq!(a, b);
    }
}
