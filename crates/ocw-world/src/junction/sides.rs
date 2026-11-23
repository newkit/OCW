//! The sides of a junction surface in plan (spec §4.8.8 *Rule: the sides of
//! a road are as smooth as the road*, J5): the outline between two mouths
//! follows the two roads' real edge lines – the offsets of their curving
//! centre lines, round on the outside of their bends – from each mouth
//! corner inward and turns from the one into the other along a rounded
//! corner (a fillet tangent to both, the corner radius of 4.8.8 Q4), so it
//! leaves every mouth corner along its arm's edge and never bends sharply.
//! Where the two edge lines do not meet (the outer side of a bend, a width
//! change, two roads of a cluster side by side) a smooth join (a cubic
//! curve tangent to both) takes the corner's place.

use super::{Arm, P2, add, cross, cumulative, dot, frame_at, mul, norm, side_pos, sub, unit};
use crate::graph::RoadGraph;

/// A side's bend between two neighbouring outline points at most (deg).
pub const SIDE_STEP_DEG: f64 = 8.0;

/// Straight pieces of a side are cut into pieces of at most this (m), as
/// the star mesh wants (`OUTLINE_STEP_M`).
const PIECE_M: f64 = 1.5;

/// Rotates `v` by `a` radians (towards [`side_pos`] for `a > 0`).
fn rot(v: P2, a: f64) -> P2 {
    let (s, c) = a.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

/// The signed angle from `a` to `b` (radians, positive towards
/// [`side_pos`]).
fn turn(a: P2, b: P2) -> f64 {
    cross(a, b).atan2(dot(a, b))
}

/// The edge line of `arm` on its `sign` side (+1: the side its polar angle
/// grows to, where [`Arm::q`] lies; −1: [`Arm::p`]'s), from its mouth
/// corner inward to its vertex: the offset of the edge's centre line by
/// the half width, with a mitre on the inner side of a bend and a round
/// join on the outer side. The first point is the mouth corner itself.
pub(super) fn edge_line(g: &RoadGraph, arm: &Arm, sign: f64) -> Vec<P2> {
    let e = &g.edges[arm.edge];
    let cum = cumulative(&e.points);
    // The centre line outward, by metres from the vertex.
    let mut ms: Vec<f64> = vec![0.0];
    for &c in &cum {
        let m = if arm.at_from { c } else { arm.len - c };
        if m > 1e-6 && m < arm.mouth_m - 1e-6 {
            ms.push(m);
        }
    }
    ms.push(arm.mouth_m);
    ms.sort_by(f64::total_cmp);
    let mut cs: Vec<P2> = Vec::with_capacity(ms.len());
    for &m in &ms {
        let p = frame_at(&e.points, &cum, arm.s_edge(m)).0;
        if cs.last().is_none_or(|q| norm(sub(p, *q)) > 1e-4) {
            cs.push(p);
        }
    }
    let corner = super::xz(if sign > 0.0 { arm.q() } else { arm.p() });
    if cs.len() < 2 {
        return vec![corner, arm.origin];
    }
    // The strip's direction past the mouth (its row is square to it): the
    // edge line bends there too where the mouth lies on a bend of the
    // centre line.
    let cm = cs[cs.len() - 1];
    cs.push(add(cm, arm.t));
    let off = |d: P2| mul(side_pos(d), sign * arm.half);
    let dirs: Vec<P2> = cs.windows(2).map(|w| unit(sub(w[1], w[0]))).collect();
    let mut out: Vec<P2> = vec![add(cs[0], off(dirs[0]))];
    for k in 1..cs.len() - 1 {
        let (d0, d1) = (dirs[k - 1], dirs[k]);
        let a = turn(d0, d1);
        if a.abs() < 1e-6 {
            continue;
        }
        let c = cs[k];
        if a * sign > 0.0 {
            // The inner side of the bend: the offset lines' crossing.
            let den = (1.0 + dot(d0, d1)).max(0.25);
            let mitre = add(c, mul(add(off(d0), off(d1)), 1.0 / den));
            if k == cs.len() - 2 {
                // At the mouth the crossing lies past the mouth corner: the
                // line leaves the corner and meets the edge coming in a
                // little inward (the bend rounded below).
                let ahead = dot(sub(mitre, corner), d1).max(0.0);
                out.push(sub(mitre, mul(d0, ahead + 0.5)));
            } else {
                out.push(mitre);
            }
        } else {
            // The outer side: round round the centre line's point.
            let n0 = off(d0);
            let steps = (a.abs() / SIDE_STEP_DEG.to_radians()).ceil().max(1.0) as usize;
            for s in 0..=steps {
                out.push(add(c, rot(n0, a * s as f64 / steps as f64)));
            }
        }
    }
    out.push(corner);
    out.reverse();
    // Past the mouth corner nothing (the join at the mouth may have put
    // points of the strip's own side there).
    while out.len() > 2 && dot(sub(out[1], corner), arm.t) > 1e-6 {
        out.remove(1);
    }
    out.dedup_by(|a, b| norm(sub(*a, *b)) < 1e-4);
    out
}

/// Arc lengths along a polyline.
fn lengths(p: &[P2]) -> Vec<f64> {
    cumulative(p)
}

/// The point `s` along polyline `p` (with its lengths `cum`) and the
/// direction there (past the ends: straight on).
fn at_len(p: &[P2], cum: &[f64], s: f64) -> (P2, P2) {
    let n = p.len();
    let k = cum.partition_point(|&c| c <= s).clamp(1, n - 1);
    let (a, b) = (p[k - 1], p[k]);
    let d = unit(sub(b, a));
    (add(a, mul(d, s - cum[k - 1])), d)
}

/// The first crossing of two polylines walked from their first points:
/// (length along `a`, along `b`, the point), the one with the least sum.
fn first_crossing(a: &[P2], b: &[P2]) -> Option<(f64, f64, P2)> {
    let (ca, cb) = (lengths(a), lengths(b));
    let mut best: Option<(f64, f64, P2)> = None;
    for i in 0..a.len().saturating_sub(1) {
        let (p, r) = (a[i], sub(a[i + 1], a[i]));
        for j in 0..b.len().saturating_sub(1) {
            let (q, s) = (b[j], sub(b[j + 1], b[j]));
            let den = cross(r, s);
            if den.abs() < 1e-12 {
                continue;
            }
            let qp = sub(q, p);
            let t = cross(qp, s) / den;
            let u = cross(qp, r) / den;
            if (-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&u) {
                let la = ca[i] + t * norm(r);
                let lb = cb[j] + u * norm(s);
                if best.is_none_or(|x| la + lb < x.0 + x.1) {
                    best = Some((la, lb, add(p, mul(r, t))));
                }
            }
        }
    }
    best
}

/// A cubic Bézier from `a` (leaving along `ua`) to `b` (arriving along
/// `ub`) with handles `ha`, `hb`, sampled so that neighbouring pieces bend
/// by at most [`SIDE_STEP_DEG`] (both ends excluded).
fn bezier(a: P2, ua: P2, b: P2, ub: P2, ha: f64, hb: f64) -> Vec<P2> {
    let (c1, c2) = (add(a, mul(ua, ha)), sub(b, mul(ub, hb)));
    let at = |t: f64| {
        let s = 1.0 - t;
        let k = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
        [
            k[0] * a[0] + k[1] * c1[0] + k[2] * c2[0] + k[3] * b[0],
            k[0] * a[1] + k[1] * c1[1] + k[2] * c2[1] + k[3] * b[1],
        ]
    };
    // The total turning, from a fine sampling.
    let fine: Vec<P2> = (0..=64).map(|i| at(i as f64 / 64.0)).collect();
    let mut total = 0.0;
    let mut prev = ua;
    for w in fine.windows(2) {
        let d = sub(w[1], w[0]);
        if norm(d) > 1e-9 {
            let d = unit(d);
            total += turn(prev, d).abs();
            prev = d;
        }
    }
    total += turn(prev, ub).abs();
    let n = ((total / SIDE_STEP_DEG.to_radians()).ceil() as usize).clamp(2, 64);
    (1..n).map(|i| at(i as f64 / n as f64)).collect()
}

/// The handle length of a cubic Bézier that follows a circular arc of
/// `angle` between two points `chord` apart (1/3 of the chord when
/// straight).
fn handle(chord: f64, angle: f64) -> f64 {
    let a = angle.abs();
    if a < 1e-3 {
        return chord / 3.0;
    }
    let r = chord / (2.0 * (a / 2.0).sin()).max(1e-9);
    (4.0 / 3.0) * (a / 4.0).tan() * r
}

/// The outline from arm `ai`'s corner [`Arm::q`] to arm `aj`'s corner
/// [`Arm::p`] (both included): along their edge lines and round a corner
/// of `radius`, or a smooth join where the edge lines do not meet. `None`
/// where it would not be one piece (both mouth corners the same point).
/// Two arms of a cluster's different vertices (`centre`: the cluster's)
/// whose edge lines do not meet go round the centre from mouth to mouth,
/// leaving each mouth along its arm (the edges inside the cluster lie
/// between them).
pub(super) fn corner_chain(
    g: &RoadGraph,
    ai: &Arm,
    aj: &Arm,
    radius: f64,
    centre: Option<P2>,
) -> Option<Vec<P2>> {
    let li = edge_line(g, ai, 1.0);
    let lj = edge_line(g, aj, -1.0);
    let (q, p) = (li[0], lj[0]);
    if norm(sub(q, p)) < 1e-3 || li.len() < 2 || lj.len() < 2 {
        return None;
    }
    if let Some(c) = centre
        && first_crossing(&li, &lj).is_none()
    {
        return round_centre(c, q, ai.t, p, aj.t);
    }
    let (ci, cj) = (lengths(&li), lengths(&lj));
    let mut pts: Vec<P2> = vec![q];
    // The edge line `l` from its start to `s` along it (interior points).
    let walk = |l: &[P2], c: &[f64], s: f64, out: &mut Vec<P2>| {
        for (k, &x) in l.iter().enumerate().skip(1) {
            if c[k] < s - 1e-6 {
                out.push(x);
            }
        }
    };
    let crossing = first_crossing(&li, &lj).or_else(|| {
        // Concave but the lines end before they meet (a cluster's arms,
        // short curving arms): their straight continuation past the
        // vertex.
        let reach = 2.0 * (ai.half + aj.half) + 2.0;
        let ext = |l: &[P2]| {
            let n = l.len();
            let d = unit(sub(l[n - 1], l[n - 2]));
            let mut e = l.to_vec();
            e.push(add(l[n - 1], mul(d, reach)));
            e
        };
        let (ei, ej) = (ext(&li), ext(&lj));
        let x = first_crossing(&ei, &ej)?;
        // Only a corner that opens towards the junction (inward along the
        // one, outward along the other: turning away from the side the
        // polar angle grows to).
        let (_, di) = at_len(&ei, &lengths(&ei), x.0);
        let (_, dj) = at_len(&ej, &lengths(&ej), x.1);
        (turn(di, mul(dj, -1.0)) < 0.0).then_some(x)
    });
    match crossing {
        Some((a, b, _)) => {
            let (ei, ej) = (
                extended(&li, a + 1e-6, ai.half + aj.half),
                extended(&lj, b + 1e-6, ai.half + aj.half),
            );
            let (ce, cf) = (lengths(&ei), lengths(&ej));
            let (_, din) = at_len(&ei, &ce, a);
            let (_, dj_in) = at_len(&ej, &cf, b);
            let dout = mul(dj_in, -1.0);
            let ang = turn(din, dout);
            let t = if ang.abs() < 1e-4 {
                0.0
            } else {
                radius * (ang.abs() / 2.0).tan()
            }
            .min(a)
            .min(b);
            walk(&ei, &ce, a - t, &mut pts);
            let (ti, ui) = at_len(&ei, &ce, a - t);
            let (tj, uj_in) = at_len(&ej, &cf, b - t);
            let uj = mul(uj_in, -1.0);
            if norm(sub(ti, *pts.last().expect("q"))) > 1e-4 {
                pts.push(ti);
            }
            if t > 1e-4 {
                let chord = norm(sub(tj, ti));
                let h = handle(chord, turn(ui, uj));
                pts.extend(bezier(ti, ui, tj, uj, h, h));
            }
            pts.push(tj);
            let mut back = Vec::new();
            walk(&ej, &cf, b - t, &mut back);
            back.reverse();
            pts.extend(back);
        }
        None => {
            // A smooth join of the two edge lines' inner ends; nearly
            // straight on (a width change), back from the ends so that it
            // tapers.
            let (ni, nj) = (li.len(), lj.len());
            let ui = unit(sub(li[ni - 1], li[ni - 2]));
            let uj = unit(sub(lj[nj - 2], lj[nj - 1]));
            let gap = norm(sub(lj[nj - 1], li[ni - 1]));
            let ang = turn(ui, uj);
            // Nearly straight on (a width change): one smooth taper from
            // mouth to mouth.
            let back = if ang.abs() < 30f64.to_radians() {
                ci[ni - 1].min(cj[nj - 1])
            } else {
                0.0
            };
            let _ = gap;
            let (sa, sb) = if back > 0.0 {
                (0.0, 0.0)
            } else {
                (ci[ni - 1], cj[nj - 1])
            };
            walk(&li, &ci, sa, &mut pts);
            let (a, ua) = at_len(&li, &ci, sa);
            let (b, ub_in) = at_len(&lj, &cj, sb);
            let ub = mul(ub_in, -1.0);
            if norm(sub(a, *pts.last().expect("q"))) > 1e-4 {
                pts.push(a);
            }
            let chord = norm(sub(b, a));
            if chord > 1e-3 {
                let h = handle(chord, turn(ua, ub));
                pts.extend(bezier(a, ua, b, ub, h, h));
                pts.push(b);
            }
            let mut rest = Vec::new();
            walk(&lj, &cj, sb, &mut rest);
            rest.reverse();
            pts.extend(rest);
        }
    }
    pts.push(p);
    pts.dedup_by(|a, b| norm(sub(*a, *b)) < 1e-4);
    let pts = rounded(&pts);
    // Long straight pieces cut for the mesh.
    let mut dense: Vec<P2> = vec![pts[0]];
    for w in pts.windows(2) {
        let k = (norm(sub(w[1], w[0])) / PIECE_M).ceil().max(1.0) as usize;
        for s in 1..=k {
            dense.push(add(w[0], mul(sub(w[1], w[0]), s as f64 / k as f64)));
        }
    }
    Some(dense)
}

/// `l` with a straight continuation past its end where `s` lies beyond it.
fn extended(l: &[P2], s: f64, reach: f64) -> Vec<P2> {
    let c = lengths(l);
    let n = l.len();
    if s < c[n - 1] {
        return l.to_vec();
    }
    let d = unit(sub(l[n - 1], l[n - 2]));
    let mut e = l.to_vec();
    e.push(add(l[n - 1], mul(d, s - c[n - 1] + reach)));
    e
}

/// Whether the points' polar angles round `c` grow strictly from the first
/// to the last, less than half a turn per piece (the star mesh's rule).
pub(super) fn polar_ok(c: P2, pts: &[P2]) -> bool {
    let mut total = 0.0;
    for w in pts.windows(2) {
        let (a, b) = (sub(w[0], c), sub(w[1], c));
        if norm(a) < 0.3 || norm(b) < 0.3 {
            return false;
        }
        let d = turn(a, b);
        if d <= 1e-6 {
            return false;
        }
        total += d;
    }
    total < std::f64::consts::TAU - 1e-6
}

/// The bends of a polyline above [`SIDE_STEP_DEG`] rounded (its ends and
/// their directions kept): the stretch round each such point replaced by a
/// curve tangent to the polyline at both ends, reaching [`ROUND_MAX_M`]
/// along it each way and at most 0.45 of the way to the next such bend
/// (all of it to an end).
fn rounded(p: &[P2]) -> Vec<P2> {
    let n = p.len();
    if n < 3 {
        return p.to_vec();
    }
    let cum = lengths(p);
    let total = cum[n - 1];
    let bends: Vec<usize> = (1..n - 1)
        .filter(|&k| turn(sub(p[k], p[k - 1]), sub(p[k + 1], p[k])).abs() > SIDE_STEP_DEG.to_radians())
        .collect();
    if bends.is_empty() {
        return p.to_vec();
    }
    // The stretch [s0, s1] each bend is rounded over.
    let mut spans: Vec<(f64, f64)> = Vec::with_capacity(bends.len());
    for (i, &k) in bends.iter().enumerate() {
        let before = if i == 0 {
            0.9 * cum[k]
        } else {
            0.45 * (cum[k] - cum[bends[i - 1]])
        };
        let after = if i + 1 == bends.len() {
            0.9 * (total - cum[k])
        } else {
            0.45 * (cum[bends[i + 1]] - cum[k])
        };
        let t = before.min(after).min(ROUND_MAX_M);
        spans.push((cum[k] - t, cum[k] + t));
    }
    let mut out: Vec<P2> = vec![p[0]];
    let mut k = 1;
    for &(s0, s1) in &spans {
        while k < n && cum[k] < s0 - 1e-9 {
            out.push(p[k]);
            k += 1;
        }
        let (a, ua) = at_len(p, &cum, s0);
        let (b, _) = at_len(p, &cum, s1);
        // The direction leaving `b`: of the piece `b` lies on.
        let kb = cum.partition_point(|&c| c <= s1 + 1e-9).clamp(1, n - 1);
        let ub = unit(sub(p[kb], p[kb - 1]));
        if norm(sub(a, *out.last().expect("start"))) > 1e-4 {
            out.push(a);
        }
        let chord = norm(sub(b, a));
        if chord > 1e-4 {
            let h = handle(chord, turn(ua, ub));
            out.extend(bezier(a, ua, b, ub, h, h));
            out.push(b);
        }
        while k < n && cum[k] <= s1 + 1e-9 {
            k += 1;
        }
    }
    while k < n {
        out.push(p[k]);
        k += 1;
    }
    out.dedup_by(|a, b| norm(sub(*a, *b)) < 1e-4);
    out
}

/// A bend of a side is rounded over at most this much of each piece (m).
const ROUND_MAX_M: f64 = 2.0;

/// Round the centre `c` from `q` (a mouth corner of an arm going out along
/// `tq`) to `p` (of an arm going out along `tp`), the radius changing
/// evenly, leaving and reaching the mouths along their arms; `None` where
/// it would go more than half a turn.
fn round_centre(c: P2, q: P2, tq: P2, p: P2, tp: P2) -> Option<Vec<P2>> {
    let (a, b) = (sub(q, c), sub(p, c));
    let (r0, r1) = (norm(a), norm(b));
    let span = turn(a, b);
    if span <= 0.0 || r0 < 1e-6 {
        return None;
    }
    // The arc between a little past the mouths; each mouth joined to it by
    // a curve leaving the mouth along its arm.
    let gap = |r: f64| (2.0 / r).min(span / 4.0);
    let (f0, f1) = (gap(r0) / span, 1.0 - gap(r1) / span);
    let arc = |f: f64| add(c, mul(unit(rot(a, span * f)), r0 + (r1 - r0) * f));
    let steps = ((span * (f1 - f0)) / SIDE_STEP_DEG.to_radians()).ceil().max(1.0) as usize;
    let mid: Vec<P2> = (0..=steps)
        .map(|k| arc(f0 + (f1 - f0) * k as f64 / steps as f64))
        .collect();
    let tangent = |f: f64| unit(sub(arc(f + 1e-4), arc(f - 1e-4)));
    let mut pts = vec![q];
    let (s, us) = (mid[0], tangent(f0));
    let h = handle(norm(sub(s, q)), turn(mul(tq, -1.0), us));
    pts.extend(bezier(q, mul(tq, -1.0), s, us, h, h));
    pts.extend(mid.iter().copied());
    let (e, ue) = (mid[mid.len() - 1], tangent(f1));
    let h = handle(norm(sub(p, e)), turn(ue, tp));
    pts.extend(bezier(e, ue, p, tp, h, h));
    pts.push(p);
    pts.dedup_by(|a, b| norm(sub(*a, *b)) < 1e-4);
    let mut dense: Vec<P2> = vec![pts[0]];
    for w in pts.windows(2) {
        let k = (norm(sub(w[1], w[0])) / PIECE_M).ceil().max(1.0) as usize;
        for s in 1..=k {
            dense.push(add(w[0], mul(sub(w[1], w[0]), s as f64 / k as f64)));
        }
    }
    Some(dense)
}
