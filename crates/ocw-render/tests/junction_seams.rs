//! Junction seams, measured (investigation 2026-11-15, spec §4.4 "Smooth
//! junctions"; bug: "junctions of roads … have bumps and quirks, the road
//! segments just seem to be slammed together").
//!
//! ANALYSIS / FUTURE REGRESSION TEST – not a production test yet: it reads
//! real world packages (`OCW_JUNCTION_WORLD=a.ocwpkg[:b.ocwpkg…]`; skipped
//! without) and only prints. When the junction surface of the plan lands,
//! the thresholds at the end turn into asserts (no visible step > 2 cm, no
//! gap, no verge on a carriageway at any junction of the test packages).
//!
//! For every graph vertex where two or more roads on the ground meet (no
//! bridge or tunnel), the strips of those roads are meshed exactly as the
//! renderer meshes them (`road_mesh::road_meshes`, one strip at a time, the
//! package terrain beside them) and the junction disc (widest half width +
//! 2.5 m) is sampled every 0.2 m from above:
//! - **overlap Δh**: where carriageways of two different roads lie over each
//!   other, their height difference (the lower one is hidden);
//! - **visible step**: between neighbouring samples whose top carriageway
//!   belongs to different roads, the jump of the top surface;
//! - **z-fight**: overlapping carriageways of two roads less than 1 cm
//!   apart (they flicker at a distance; markings of one show through);
//! - **gap**: inside a road's outline (half width − 0.1 m around its
//!   centreline, round at the vertex) but under no carriageway (the wedge
//!   on the outside of a bend between two square ends);
//! - **verge on road**: a shoulder, kerb or sidewalk top of one road above
//!   another road's carriageway by more than 1 cm;
//! - **markings**: minor roads whose centre or edge lines run on into the
//!   junction (the strip ends at the vertex, on the major road).
//!
//! Run (on a build machine): `OCW_JUNCTION_WORLD=… cargo test --release
//! -p ocw-render --test junction_seams -- --nocapture`.

use std::collections::HashMap;

use bevy::math::{Vec2, Vec3, Vec3Swizzles};
use ocw_render::road_mesh::{Kind, RoadContext, decode_b, road_meshes};
use ocw_world::geo::LocalProjection;
use ocw_world::graph::RoadGraph;
use ocw_world::road_dress::{MarkCode, MarkingStyle};
use ocw_world::WorldPackage;

const SAMPLE_M: f32 = 0.2;
const MARGIN_M: f32 = 2.5;
const CELL_M: f32 = 1.0;

/// Up-facing triangles near one junction, by incident edge, with their part.
#[derive(Default)]
struct Junction {
    v: usize,
    tris: Vec<([Vec3; 3], usize, u8)>,
}

fn tri_height(t: &[Vec3; 3], p: Vec2) -> Option<f32> {
    let (a, b, c) = (t[0].xz(), t[1].xz(), t[2].xz());
    let d = (b - a).perp_dot(c - a);
    if d.abs() < 1e-9 {
        return None;
    }
    let w1 = (p - a).perp_dot(c - a) / d;
    let w2 = (b - a).perp_dot(p - a) / d;
    let w0 = 1.0 - w1 - w2;
    let e = -1e-4;
    (w0 >= e && w1 >= e && w2 >= e).then(|| w0 * t[0].y + w1 * t[1].y + w2 * t[2].y)
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    (((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt(), t)
}

fn poly_dist(p: [f64; 2], pts: &[[f64; 2]]) -> f64 {
    pts.windows(2)
        .map(|w| seg_dist(p, w[0], w[1]).0)
        .fold(f64::INFINITY, f64::min)
}

/// Grade of an edge's first (or last) profile interval, leaving `v`.
fn grade_from(g: &RoadGraph, ei: usize, v: usize) -> f64 {
    let e = &g.edges[ei];
    let p = &e.profile;
    if p.len() < 2 {
        return 0.0;
    }
    let len = e.length_m().min(ocw_world::graph::PROFILE_STEP_M).max(1e-6);
    if e.from.0 as usize == v {
        (p[1] - p[0]) / len
    } else {
        (p[p.len() - 2] - p[p.len() - 1]) / len
    }
}

#[derive(Default, Clone)]
struct Result {
    v: usize,
    degree: usize,
    bend_deg: f64,
    overlap_dh: f32,
    step: f32,
    zfight_m2: f32,
    gap_m2: f32,
    verge_m2: f32,
    verge_dh: f32,
    lines_in: usize,
    major_grade: f64,
    max_grade: f64,
}

fn analyse(path: &str) {
    let pkg = WorldPackage::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let g = pkg.graph.clone().expect("road graph");
    let proj = LocalProjection::new(pkg.origin.0, pkg.origin.1);
    let adj = g.adjacency();
    // Junctions: degree >= 2, all on the ground, not at the region border.
    let mut jidx: HashMap<usize, usize> = HashMap::new();
    let mut juncs: Vec<Junction> = Vec::new();
    for (v, edges) in adj.iter().enumerate() {
        if edges.len() < 2
            || g.boundary_vertices
                .contains(&ocw_world::graph::VertexId(v as u32))
            || edges.iter().any(|&e| g.edges[e].bridge || g.edges[e].tunnel)
            || edges.iter().any(|&e| g.edges[e].from == g.edges[e].to)
        {
            continue;
        }
        jidx.insert(v, juncs.len());
        juncs.push(Junction {
            v,
            ..Default::default()
        });
    }
    // Edge segments on a 20 m grid (strip -> edge).
    let mut grid: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    let k = |x: f64| (x / 20.0).floor() as i32;
    for (ei, e) in g.edges.iter().enumerate() {
        for (si, w) in e.points.windows(2).enumerate() {
            let (lo, hi) = (
                [w[0][0].min(w[1][0]), w[0][1].min(w[1][1])],
                [w[0][0].max(w[1][0]), w[0][1].max(w[1][1])],
            );
            for x in k(lo[0])..=k(hi[0]) {
                for z in k(lo[1])..=k(hi[1]) {
                    grid.entry((x, z)).or_default().push((ei, si));
                }
            }
        }
    }
    let nearest_edge = |p: [f64; 2]| -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &(ei, si) in grid.get(&(k(p[0]) + dx, k(p[1]) + dz)).into_iter().flatten() {
                    let w = &g.edges[ei].points[si..si + 2];
                    let d = seg_dist(p, w[0], w[1]).0;
                    if best.is_none_or(|b| d < b.1) {
                        best = Some((ei, d));
                    }
                }
            }
        }
        best
    };
    let radius = |v: usize| -> f32 {
        adj[v]
            .iter()
            .map(|&e| g.edges[e].width_m as f32 / 2.0)
            .fold(0.0, f32::max)
            + MARGIN_M
    };
    let style = MarkingStyle::default();
    let ground = |x: f32, z: f32| pkg.surface_height(f64::from(x), f64::from(z)).map(|h| h as f32);
    let ctx = RoadContext {
        colours: None,
        ground: &ground,
        deck: &|_| false,
        style: &style,
    };
    let mut lines_in: HashMap<usize, usize> = HashMap::new();
    let mut unmatched = 0usize;
    for tile in pkg.tiles.values() {
        for s in &tile.roads {
            let n = s.points.len();
            if n < 2 {
                continue;
            }
            let m = (n - 1) / 2;
            let mid = [
                f64::from(s.points[m][0] + s.points[m + 1][0]) / 2.0,
                f64::from(s.points[m][2] + s.points[m + 1][2]) / 2.0,
            ];
            let Some((ei, d)) = nearest_edge(mid) else {
                unmatched += 1;
                continue;
            };
            if d > 0.2 {
                unmatched += 1;
                continue;
            }
            let e = &g.edges[ei];
            let ends: Vec<usize> = [e.from.0 as usize, e.to.0 as usize]
                .into_iter()
                .filter(|v| jidx.contains_key(v))
                .collect();
            if ends.is_empty() {
                continue;
            }
            // Near which junction ends is this strip at all?
            let near: Vec<usize> = ends
                .into_iter()
                .filter(|&v| {
                    let c = Vec2::new(g.vertices[v][0] as f32, g.vertices[v][1] as f32);
                    let r = radius(v) + 3.0;
                    s.points.iter().any(|p| Vec2::new(p[0], p[2]).distance(c) < r + 6.0)
                })
                .collect();
            if near.is_empty() {
                continue;
            }
            // Markings running into a junction from a minor road's end.
            let code = MarkCode::of(&s.dress, s.class, s.surface).lines_code();
            let painted = (code >> 6) & 7 != 0 || (code >> 9) & 1 == 1;
            let mesh = road_meshes(std::slice::from_ref(s), &ctx).roads;
            for &v in &near {
                let c = Vec2::new(g.vertices[v][0] as f32, g.vertices[v][1] as f32);
                let r = radius(v);
                if painted && adj[v].len() >= 3 {
                    let ends_here = [s.points[0], s.points[n - 1]]
                        .iter()
                        .any(|p| Vec2::new(p[0], p[2]).distance(c) < 0.1);
                    let major = adj[v]
                        .iter()
                        .map(|&o| g.edges[o].width_m)
                        .fold(0.0, f64::max);
                    if ends_here && e.width_m < major - 0.01 {
                        *lines_in.entry(v).or_default() += 1;
                    }
                }
                let j = &mut juncs[jidx[&v]];
                for t in mesh.indices.chunks_exact(3) {
                    let p = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
                    let up = (p[1] - p[0]).cross(p[2] - p[0]);
                    if up.y.abs() < 1e-5 {
                        continue;
                    }
                    if p.iter().all(|q| q.xz().distance(c) > r + 3.0) {
                        continue;
                    }
                    let (_, kind, _) = decode_b(mesh.uvs_b[t[0] as usize][0]);
                    j.tris.push((p, ei, kind));
                }
            }
        }
    }
    let mut results: Vec<Result> = Vec::new();
    for j in &juncs {
        let v = j.v;
        let c = Vec2::new(g.vertices[v][0] as f32, g.vertices[v][1] as f32);
        let r = radius(v);
        // Triangle buckets.
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let ck = |x: f32| (x / CELL_M).floor() as i32;
        for (i, (t, _, _)) in j.tris.iter().enumerate() {
            let lo = t[0].min(t[1]).min(t[2]);
            let hi = t[0].max(t[1]).max(t[2]);
            for x in ck(lo.x)..=ck(hi.x) {
                for z in ck(lo.z)..=ck(hi.z) {
                    cells.entry((x, z)).or_default().push(i as u32);
                }
            }
        }
        let steps = (2.0 * r / SAMPLE_M).ceil() as i32;
        let w = (steps + 1) as usize;
        // Per sample: the top carriageway (edge, h).
        let mut top: Vec<Option<(usize, f32)>> = vec![None; w * w];
        let mut res = Result {
            v,
            degree: adj[v].len(),
            lines_in: lines_in.get(&v).copied().unwrap_or(0),
            ..Default::default()
        };
        let a = SAMPLE_M * SAMPLE_M;
        for iz in 0..=steps {
            for ix in 0..=steps {
                let p = c + Vec2::new(
                    -r + ix as f32 * SAMPLE_M,
                    -r + iz as f32 * SAMPLE_M,
                );
                if p.distance(c) > r {
                    continue;
                }
                let mut surf: Vec<(usize, f32)> = Vec::new();
                let mut verge: Vec<(usize, f32)> = Vec::new();
                for &ti in cells.get(&(ck(p.x), ck(p.y))).into_iter().flatten() {
                    let (t, ei, kind) = &j.tris[ti as usize];
                    let Some(h) = tri_height(t, p) else { continue };
                    if *kind == Kind::Surface as u8 {
                        match surf.iter_mut().find(|s| s.0 == *ei) {
                            Some(s) => s.1 = s.1.max(h),
                            None => surf.push((*ei, h)),
                        }
                    } else if [Kind::Shoulder as u8, Kind::Sidewalk as u8, Kind::Kerb as u8]
                        .contains(kind)
                    {
                        verge.push((*ei, h));
                    }
                }
                let t = surf.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1));
                top[iz as usize * w + ix as usize] = t;
                if surf.len() >= 2 {
                    let lo = surf.iter().map(|s| s.1).fold(f32::INFINITY, f32::min);
                    let hi = surf.iter().map(|s| s.1).fold(f32::NEG_INFINITY, f32::max);
                    res.overlap_dh = res.overlap_dh.max(hi - lo);
                    if hi - lo < 0.01 {
                        res.zfight_m2 += a;
                    }
                }
                if surf.is_empty() {
                    let q = [f64::from(p.x), f64::from(p.y)];
                    let inside = adj[v].iter().any(|&ei| {
                        let e = &g.edges[ei];
                        poly_dist(q, &e.points) < e.width_m / 2.0 - 0.1
                    });
                    if inside {
                        res.gap_m2 += a;
                    }
                }
                if let Some((te, th)) = t {
                    let worst = verge
                        .iter()
                        .filter(|(ve, vh)| *ve != te && *vh > th + 0.01)
                        .map(|(_, vh)| vh - th)
                        .fold(0.0, f32::max);
                    if worst > 0.0 {
                        res.verge_m2 += a;
                        res.verge_dh = res.verge_dh.max(worst);
                    }
                }
            }
        }
        for iz in 0..=steps as usize {
            for ix in 0..=steps as usize {
                let Some((e0, h0)) = top[iz * w + ix] else { continue };
                for (jx, jz) in [(ix + 1, iz), (ix, iz + 1)] {
                    if jx > steps as usize || jz > steps as usize {
                        continue;
                    }
                    if let Some((e1, h1)) = top[jz * w + jx]
                        && e1 != e0
                    {
                        res.step = res.step.max((h1 - h0).abs());
                    }
                }
            }
        }
        // Bend at a plain split; grades.
        let dirs: Vec<[f64; 2]> = adj[v]
            .iter()
            .map(|&ei| {
                let e = &g.edges[ei];
                let pts: Vec<[f64; 2]> = if e.from.0 as usize == v {
                    e.points.clone()
                } else {
                    e.points.iter().rev().copied().collect()
                };
                let (a, b) = (pts[0], pts[1.min(pts.len() - 1)]);
                let l = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt().max(1e-9);
                [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
            })
            .collect();
        if dirs.len() == 2 {
            let dot = (dirs[0][0] * dirs[1][0] + dirs[0][1] * dirs[1][1]).clamp(-1.0, 1.0);
            res.bend_deg = 180.0 - dot.acos().to_degrees();
        }
        let major = adj[v]
            .iter()
            .copied()
            .max_by(|&a, &b| g.edges[a].width_m.total_cmp(&g.edges[b].width_m))
            .unwrap();
        res.major_grade = grade_from(&g, major, v);
        res.max_grade = adj[v]
            .iter()
            .map(|&e| grade_from(&g, e, v).abs())
            .fold(0.0, f64::max);
        results.push(res);
    }
    // Report.
    println!("\n=== {path}");
    println!(
        "junction vertices analysed: {} (strips unmatched to an edge: {unmatched})",
        results.len()
    );
    let bins = [0.02f32, 0.05, 0.10, 0.25, 0.50, f32::INFINITY];
    for (label, filt) in [
        ("degree 2 (way splits)", 2usize..3),
        ("degree 3 (T / Y)", 3..4),
        ("degree 4 (crossings)", 4..5),
        ("degree >= 5", 5..99),
    ] {
        let rs: Vec<&Result> = results.iter().filter(|r| filt.contains(&r.degree)).collect();
        if rs.is_empty() {
            continue;
        }
        let mut counts = [0usize; 6];
        for r in &rs {
            let b = bins.iter().position(|&b| r.step < b).unwrap();
            counts[b] += 1;
        }
        let mut steps: Vec<f32> = rs.iter().map(|r| r.step).collect();
        steps.sort_by(f32::total_cmp);
        let pct = |q: f32| steps[((steps.len() - 1) as f32 * q) as usize];
        let n = rs.len() as f32;
        let share = |f: &dyn Fn(&Result) -> bool| {
            100.0 * rs.iter().filter(|r| f(r)).count() as f32 / n
        };
        println!(
            "{label}: {} | visible step <2cm {} | 2-5 {} | 5-10 {} | 10-25 {} | 25-50 {} | >50 {} | median {:.3} p90 {:.3} p99 {:.3} max {:.3} m",
            rs.len(),
            counts[0],
            counts[1],
            counts[2],
            counts[3],
            counts[4],
            counts[5],
            pct(0.5),
            pct(0.9),
            pct(0.99),
            pct(1.0)
        );
        println!(
            "    overlap dh > 5 cm: {:.1} % | z-fight (< 1 cm, > 0.5 m2): {:.1} %, mean {:.2} m2 | gap > 0.1 m2: {:.1} %, mean {:.2} m2, max {:.2} | verge on road > 0.1 m2: {:.1} % (max {:.2} m above) | minor-road lines into it: {:.1} %",
            share(&|r| r.overlap_dh > 0.05),
            share(&|r| r.zfight_m2 > 0.5),
            rs.iter().map(|r| r.zfight_m2).sum::<f32>() / n,
            share(&|r| r.gap_m2 > 0.1),
            rs.iter().map(|r| r.gap_m2).sum::<f32>() / n,
            rs.iter().map(|r| r.gap_m2).fold(0.0, f32::max),
            share(&|r| r.verge_m2 > 0.1),
            rs.iter().map(|r| r.verge_dh).fold(0.0, f32::max),
            share(&|r| r.lines_in > 0),
        );
        if filt.start == 2 {
            for (lo, hi) in [(0.0, 10.0), (10.0, 30.0), (30.0, 60.0), (60.0, 181.0)] {
                let b: Vec<&&Result> =
                    rs.iter().filter(|r| r.bend_deg >= lo && r.bend_deg < hi).collect();
                if b.is_empty() {
                    continue;
                }
                println!(
                    "    bend {lo:.0}-{hi:.0} deg: {} splits, gap mean {:.2} m2, max {:.2} m2",
                    b.len(),
                    b.iter().map(|r| r.gap_m2).sum::<f32>() / b.len() as f32,
                    b.iter().map(|r| r.gap_m2).fold(0.0, f32::max)
                );
            }
        }
    }
    // Correlation: step vs the steepest road at T / crossings.
    let side: Vec<&Result> = results.iter().filter(|r| r.degree >= 3).collect();
    let flat_major = side.iter().filter(|r| r.major_grade.abs() < 0.03).count();
    println!("    degree >= 3 with the widest road under 3 %: {flat_major}");
    for (lo, hi) in [(0.0, 0.03), (0.03, 0.06), (0.06, 0.10), (0.10, 0.15), (0.15, 1.0)] {
        let b: Vec<&&Result> = side
            .iter()
            .filter(|r| r.max_grade >= lo && r.max_grade < hi)
            .collect();
        if b.is_empty() {
            continue;
        }
        let mut s: Vec<f32> = b.iter().map(|r| r.step).collect();
        s.sort_by(f32::total_cmp);
        println!(
            "    steepest road at the junction {:.0}-{:.0} %: {} junctions, step median {:.3} max {:.3} m",
            lo * 100.0,
            hi * 100.0,
            b.len(),
            s[s.len() / 2],
            s[s.len() - 1]
        );
    }
    let describe = |r: &Result| {
        let p = g.vertices[r.v];
        let (lat, lon) = proj.unproject(p);
        let roads: Vec<String> = adj[r.v]
            .iter()
            .map(|&ei| {
                let e = &g.edges[ei];
                format!(
                    "{:?} {} {:.1}m w{} {:+.0}%",
                    e.class,
                    e.name.as_deref().unwrap_or("-"),
                    e.width_m,
                    e.osm_way,
                    grade_from(&g, ei, r.v) * 100.0
                )
            })
            .collect();
        format!(
            "x {:.1} z {:.1} ({lat:.6}, {lon:.6}) deg {} step {:.3} overlap {:.3} zf {:.1} gap {:.2} verge {:.2}/{:.2} lines {} | {}",
            p[0],
            p[1],
            r.degree,
            r.step,
            r.overlap_dh,
            r.zfight_m2,
            r.gap_m2,
            r.verge_m2,
            r.verge_dh,
            r.lines_in,
            roads.join(" ; ")
        )
    };
    let mut by_step: Vec<&Result> = results.iter().filter(|r| r.degree >= 3).collect();
    by_step.sort_by(|a, b| b.step.total_cmp(&a.step));
    println!("worst visible steps (degree >= 3):");
    for r in by_step.iter().take(15) {
        println!("  {}", describe(r));
    }
    let mut by_gap: Vec<&Result> = results.iter().collect();
    by_gap.sort_by(|a, b| b.gap_m2.total_cmp(&a.gap_m2));
    println!("worst gaps:");
    for r in by_gap.iter().take(8) {
        println!("  {}", describe(r));
    }
    let mut by_verge: Vec<&Result> = results.iter().collect();
    by_verge.sort_by(|a, b| b.verge_dh.total_cmp(&a.verge_dh));
    println!("worst verges on a road:");
    for r in by_verge.iter().take(8) {
        println!("  {}", describe(r));
    }
    // Typical ones: degree 3, step near the median, for pictures.
    let mut t3: Vec<&Result> = results.iter().filter(|r| r.degree == 3).collect();
    t3.sort_by(|a, b| a.step.total_cmp(&b.step));
    println!("typical T junctions (median step):");
    for r in t3.iter().skip(t3.len() / 2).take(4) {
        println!("  {}", describe(r));
    }
}

#[test]
fn junction_seams_of_real_packages() {
    let Some(paths) = std::env::var_os("OCW_JUNCTION_WORLD") else {
        eprintln!("OCW_JUNCTION_WORLD not set: skipped");
        return;
    };
    for p in paths.to_string_lossy().split(':') {
        analyse(p);
    }
}
