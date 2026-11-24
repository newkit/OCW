//! The mesh of a junction surface whose outline is not star-shaped from
//! its centre (spec §4.8.8 Rule, J6): arms that curve strongly within the
//! junction, forks, clusters. The outline is cut into triangles by a
//! constrained Delaunay triangulation, with points inside it about
//! [`super::RING_M`] apart and two rows just inside the outline (as the
//! star mesh's two outer rings), all at the surface's heights from the
//! patch's field; lookups go through a grid of 1 m cells instead of the
//! star's sectors.

use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

use super::{OUTER_RING_M, P2, RING_M, add, cross, mul, norm, seg_dist, sub, unit, xz};

/// Cell size of the triangle index (m).
pub(super) const CELL_M: f64 = 1.0;

/// Whether the closed polyline `p` is simple (no two of its pieces cross
/// or touch, neighbours apart) and runs counter-clockwise in (x, z) as the
/// star outlines do (growing polar angle).
pub(super) fn simple_ccw(p: &[P2]) -> bool {
    let n = p.len();
    if n < 3 {
        return false;
    }
    let mut area = 0.0;
    for k in 0..n {
        area += cross(p[k], p[(k + 1) % n]);
    }
    if area <= 0.0 {
        return false;
    }
    crossing_pieces(p).is_empty()
}

/// The pieces (by their first point) of the closed polyline `p` that cross
/// or touch another piece (neighbours sharing a point apart).
pub(super) fn crossing_pieces(p: &[P2]) -> Vec<usize> {
    let n = p.len();
    let mut out = Vec::new();
    // A coarse grid of the pieces' boxes.
    let mut lo = [f64::INFINITY; 2];
    for q in p {
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
    }
    let cell = 2.0;
    let key = |x: f64, z: f64| {
        (
            ((x - lo[0]) / cell).floor() as i64,
            ((z - lo[1]) / cell).floor() as i64,
        )
    };
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for k in 0..n {
        let (a, b) = (p[k], p[(k + 1) % n]);
        let (k0, k1) = (
            key(a[0].min(b[0]), a[1].min(b[1])),
            key(a[0].max(b[0]), a[1].max(b[1])),
        );
        for x in k0.0..=k1.0 {
            for z in k0.1..=k1.1 {
                grid.entry((x, z)).or_default().push(k);
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for list in grid.values() {
        for (ia, &i) in list.iter().enumerate() {
            for &j in &list[ia + 1..] {
                let (i, j) = (i.min(j), i.max(j));
                if j == i + 1 || (i == 0 && j == n - 1) || !seen.insert((i, j)) {
                    continue;
                }
                if pieces_meet(p[i], p[(i + 1) % n], p[j], p[(j + 1) % n]) {
                    out.push(i);
                    out.push(j);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Whether the pieces a–b and c–d cross or touch.
fn pieces_meet(a: P2, b: P2, c: P2, d: P2) -> bool {
    let o = |p: P2, q: P2, r: P2| cross(sub(q, p), sub(r, p));
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    let eps = 1e-6;
    seg_dist(a, c, d) < eps
        || seg_dist(b, c, d) < eps
        || seg_dist(c, a, b) < eps
        || seg_dist(d, a, b) < eps
}

/// Whether `q` lies inside the closed polyline `p` (even-odd).
pub(super) fn inside(p: &[P2], q: P2) -> bool {
    let n = p.len();
    let mut c = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (p[i], p[j]);
        if (a[1] > q[1]) != (b[1] > q[1])
            && q[0] < (b[0] - a[0]) * (q[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            c = !c;
        }
        j = i;
    }
    c
}

/// The distance of `q` from the closed polyline `p`, the piece (by its
/// first point) and how far along it (0 – 1) its nearest point lies.
pub(super) fn nearest_piece(p: &[P2], q: P2) -> (f64, usize, f64) {
    let n = p.len();
    let mut best = (f64::INFINITY, 0, 0.0);
    for k in 0..n {
        let (a, b) = (p[k], p[(k + 1) % n]);
        let ab = sub(b, a);
        let l2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if l2 > 0.0 {
            ((sub(q, a)[0] * ab[0] + sub(q, a)[1] * ab[1]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = norm(sub(add(a, mul(ab, t)), q));
        if d < best.0 {
            best = (d, k, t);
        }
    }
    best
}

/// The points of the mesh inside the outline `p` (counter-clockwise):
/// two rows [`OUTER_RING_M`] and 2.5 × that inside each outline point
/// (along the bisector), and a grid [`RING_M`] apart – each kept only
/// where it lies inside, clear of the outline by at least 0.8 of its own
/// distance (the grid's: 0.6 × [`RING_M`]) and not within 0.3 m of a point
/// kept before.
pub(super) fn inner_points(p: &[P2]) -> Vec<P2> {
    let n = p.len();
    let mut out: Vec<P2> = Vec::new();
    let mut taken: std::collections::HashMap<(i64, i64), Vec<P2>> =
        std::collections::HashMap::new();
    let ck = |q: P2| ((q[0] / 0.3).floor() as i64, (q[1] / 0.3).floor() as i64);
    let mut keep = |q: P2, clear: f64, out: &mut Vec<P2>| {
        if !inside(p, q) || nearest_piece(p, q).0 < clear {
            return;
        }
        let k = ck(q);
        for dx in -1..=1 {
            for dz in -1..=1 {
                if taken
                    .get(&(k.0 + dx, k.1 + dz))
                    .is_some_and(|v| v.iter().any(|w| norm(sub(*w, q)) < 0.3))
                {
                    return;
                }
            }
        }
        taken.entry(k).or_default().push(q);
        out.push(q);
    };
    for d in [OUTER_RING_M, 2.5 * OUTER_RING_M] {
        for k in 0..n {
            let (a, b, c) = (p[(k + n - 1) % n], p[k], p[(k + 1) % n]);
            let (u, w) = (unit(sub(b, a)), unit(sub(c, b)));
            // Inside is to the left of a counter-clockwise outline.
            let left = |v: P2| [-v[1], v[0]];
            let bis = unit(add(left(u), left(w)));
            let s = (1.0 + u[0] * w[0] + u[1] * w[1]).max(0.25).sqrt() / std::f64::consts::SQRT_2;
            let q = add(b, mul(bis, d / s.max(0.5)));
            keep(q, 0.8 * d, &mut out);
        }
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for q in p {
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    let (x0, z0) = (
        (lo[0] / RING_M).floor() * RING_M,
        (lo[1] / RING_M).floor() * RING_M,
    );
    let mut x = x0;
    while x <= hi[0] {
        let mut z = z0;
        while z <= hi[1] {
            keep([x, z], 0.6 * RING_M, &mut out);
            z += RING_M;
        }
        x += RING_M;
    }
    out
}

/// The triangles of the outline `p` (its points first, then `extra`), by
/// index into `p` followed by `extra`; `None` where the triangulation
/// cannot hold the outline.
pub(super) fn triangulate(p: &[P2], extra: &[P2]) -> Option<Vec<[u32; 3]>> {
    let n = p.len();
    let mut cdt: ConstrainedDelaunayTriangulation<Point2<f64>> =
        ConstrainedDelaunayTriangulation::new();
    let mut handles = Vec::with_capacity(n + extra.len());
    for q in p.iter().chain(extra) {
        let h = cdt.insert(Point2::new(q[0], q[1])).ok()?;
        if h.index() != handles.len() {
            // The same point twice.
            return None;
        }
        handles.push(h);
    }
    for k in 0..n {
        let (a, b) = (handles[k], handles[(k + 1) % n]);
        if !cdt.can_add_constraint(a, b) {
            return None;
        }
        cdt.add_constraint(a, b);
    }
    let mut out = Vec::new();
    for f in cdt.inner_faces() {
        let v = f.vertices();
        let pos: Vec<P2> = v
            .iter()
            .map(|h| {
                let q = h.position();
                [q.x, q.y]
            })
            .collect();
        let c = mul(add(add(pos[0], pos[1]), pos[2]), 1.0 / 3.0);
        if !inside(p, c) {
            continue;
        }
        out.push(v.map(|h| h.fix().index() as u32));
    }
    (!out.is_empty()).then_some(out)
}

/// A grid of 1 m cells over a patch's box: the triangles touching each.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellIndex {
    pub origin: P2,
    pub nx: usize,
    pub nz: usize,
    pub cells: Vec<Vec<u32>>,
}

impl CellIndex {
    pub(super) fn of(vertices: &[[f64; 3]], triangles: &[[u32; 3]], lo: P2, hi: P2) -> Self {
        let origin = [lo[0] - 0.5, lo[1] - 0.5];
        let nx = ((hi[0] - origin[0]) / CELL_M).ceil().max(1.0) as usize + 1;
        let nz = ((hi[1] - origin[1]) / CELL_M).ceil().max(1.0) as usize + 1;
        let mut cells = vec![Vec::new(); nx * nz];
        for (ti, t) in triangles.iter().enumerate() {
            let q = t.map(|i| xz(vertices[i as usize]));
            let (mut a, mut b) = (q[0], q[0]);
            for r in &q[1..] {
                a = [a[0].min(r[0]), a[1].min(r[1])];
                b = [b[0].max(r[0]), b[1].max(r[1])];
            }
            let cx = |x: f64, m: usize| {
                (((x - origin[0]) / CELL_M).floor().max(0.0) as usize).min(m - 1)
            };
            let cz = |z: f64, m: usize| {
                (((z - origin[1]) / CELL_M).floor().max(0.0) as usize).min(m - 1)
            };
            for x in cx(a[0], nx)..=cx(b[0], nx) {
                for z in cz(a[1], nz)..=cz(b[1], nz) {
                    cells[z * nx + x].push(ti as u32);
                }
            }
        }
        Self {
            origin,
            nx,
            nz,
            cells,
        }
    }

    pub(super) fn at(&self, p: P2) -> &[u32] {
        let x = ((p[0] - self.origin[0]) / CELL_M).floor();
        let z = ((p[1] - self.origin[1]) / CELL_M).floor();
        if x < 0.0 || z < 0.0 || x as usize >= self.nx || z as usize >= self.nz {
            return &[];
        }
        &self.cells[z as usize * self.nx + x as usize]
    }
}
