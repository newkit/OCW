//! The detailed aircraft (spec 14 Phase 18, 18G *Detail from the
//! reference photos*): two narrow-body twinjets and the medium transport
//! helicopter. Proportions, shapes and details follow the user's own
//! apron photos (`docs/reference/aircraft.md`) – our own generic designs
//! and liveries, no airline, no registration, no 1:1 copy of a real type.
//!
//! The airliners: a tapered nose with the cockpit's windscreen and side
//! windows, the cabin's window row, door, emergency-exit and cargo-door
//! outlines on the fuselage's facets, the belly fairing, a swept wing in
//! two panels with an airfoil section, flap-track fairings, high-bypass
//! nacelles with the inlet lip, the fan face (spinner and blades), the
//! fan and core nozzles and the plug, pylons, blended wing-tip sharklets
//! or split winglets, the fin with its dorsal fillet, the tailplane, the
//! APU's tail cone and exhaust, nose and main gear with their doors
//! (`gear_*`: drawn when down; the closed bays are the clean belly).

use super::*;

/// The wing tips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Winglet {
    /// A blended upward tip (the larger narrow-body of `ref-1`).
    Sharklet,
    /// An upper and a lower blade (`ref-2`).
    Split,
}

/// One narrow-body twinjet.
pub(super) struct AirlinerSpec {
    /// The fuselage's centre line height and radius (m).
    cy: f32,
    r: f32,
    /// The nose tip and the tail cone's end (z).
    nose: f32,
    tail: f32,
    /// The nose: (dz from the tip, r / `r`, lift / `r`), the last one the
    /// cabin's start.
    nose_prof: &'static [(f32, f32, f32)],
    /// The tail: (dz before the end, r / `r`, lift / `r`), the first one
    /// the cabin's end.
    tail_prof: &'static [(f32, f32, f32)],
    /// The wing: root, kink and tip as [x, y, z of the leading edge, z of
    /// the trailing edge, thickness] (y relative to `cy`).
    wing: [[f32; 5]; 3],
    winglet: Winglet,
    /// The engines: the axis at (x, y), the fan face's z, the fan's
    /// radius, the nacelle's height over its width.
    eng: [f32; 5],
    /// The tailplane's root and tip ([`wing`], y relative to `cy`).
    tailplane: [[f32; 5]; 2],
    /// The fin: root y (over `cy`), its leading and trailing edge; tip y,
    /// leading and trailing edge; the dorsal fillet's start (z).
    fin: [f32; 7],
    /// The gear: the nose wheel's z and radius, the main legs' x and z,
    /// the main wheels' radius.
    gear: [f32; 5],
    /// The doors along the cabin (z of the front edge): the forward and
    /// aft passenger doors, the over-wing exits, the cargo doors.
    doors: [f32; 2],
    exits: &'static [f32],
    cargo: [f32; 2],
    body: u32,
    livery: u32,
}

/// The larger-fan twinjet with sharklets (`ref-1`'s proportions).
pub(super) const AIRLINER_SHARKLET: AirlinerSpec = AirlinerSpec {
    cy: 3.5,
    r: 1.98,
    nose: -18.8,
    tail: 18.9,
    nose_prof: &[
        (0.0, 0.0, -0.16),
        (0.15, 0.21, -0.15),
        (0.5, 0.43, -0.13),
        (1.0, 0.62, -0.09),
        (1.7, 0.78, -0.05),
        (2.5, 0.91, -0.02),
        (3.6, 0.985, 0.0),
        (4.8, 1.0, 0.0),
    ],
    tail_prof: &[
        (-9.3, 1.0, 0.0),
        (-6.8, 0.935, 0.06),
        (-4.3, 0.73, 0.21),
        (-2.2, 0.48, 0.36),
        (-0.6, 0.24, 0.49),
        (0.0, 0.13, 0.55),
    ],
    wing: [
        [1.2, -1.0, -3.9, 3.3, 0.55],
        [6.6, -0.51, -1.04, 1.7, 0.36],
        [16.6, 0.39, 4.26, 5.6, 0.13],
    ],
    winglet: Winglet::Sharklet,
    eng: [5.75, 1.42, -3.9, 1.05, 1.0],
    tailplane: [[0.9, 0.45, 14.2, 17.4, 0.3], [6.2, 0.85, 17.3, 18.5, 0.1]],
    fin: [1.2, 12.4, 18.4, 8.25, 16.4, 18.45, 10.4],
    gear: [-12.4, 0.38, 3.8, 1.0, 0.6],
    doors: [-13.5, 7.9],
    exits: &[-1.25, -0.35],
    cargo: [-9.2, 4.6],
    body: 0xF2F3F4,
    livery: 0x284B8C,
};

/// The longer twinjet with split winglets and flatter, lower-slung
/// nacelles (`ref-2`'s proportions).
pub(super) const AIRLINER_SPLIT: AirlinerSpec = AirlinerSpec {
    cy: 3.3,
    r: 1.9,
    nose: -19.6,
    tail: 19.4,
    nose_prof: &[
        (0.0, 0.0, -0.2),
        (0.2, 0.2, -0.19),
        (0.7, 0.42, -0.16),
        (1.4, 0.62, -0.11),
        (2.2, 0.78, -0.06),
        (3.1, 0.9, -0.025),
        (4.2, 0.98, 0.0),
        (5.4, 1.0, 0.0),
    ],
    tail_prof: &[
        (-10.0, 1.0, 0.0),
        (-7.4, 0.93, 0.07),
        (-4.6, 0.72, 0.22),
        (-2.3, 0.47, 0.37),
        (-0.6, 0.22, 0.5),
        (0.0, 0.12, 0.54),
    ],
    wing: [
        [1.15, -0.95, -3.6, 3.4, 0.55],
        [6.2, -0.5, -0.95, 1.8, 0.36],
        [17.4, 0.48, 4.9, 6.1, 0.12],
    ],
    winglet: Winglet::Split,
    eng: [5.0, 1.36, -4.0, 0.98, 0.84],
    tailplane: [[0.85, 0.5, 14.6, 17.8, 0.3], [6.4, 0.9, 17.9, 19.0, 0.1]],
    fin: [1.2, 12.0, 18.8, 8.7, 16.6, 18.9, 8.8],
    gear: [-14.0, 0.36, 3.5, 1.4, 0.56],
    doors: [-14.2, 8.6],
    exits: &[-0.6],
    cargo: [-10.0, 5.2],
    body: 0xEEF0F2,
    livery: 0x0F8A9D,
};

/// The corners of facet `j` of the band from profile point `a` to `b`:
/// ring `a`'s vertices `j`, `j + 1`, then ring `b`'s `j + 1`, `j`.
fn facet(l: &Lathe, a: (f32, f32, f32), b: (f32, f32, f32), j: u32) -> [Vec3; 4] {
    let k = (j + 1) % l.sides;
    [
        l.point(a.0, a.1, a.2, j),
        l.point(a.0, a.1, a.2, k),
        l.point(b.0, b.1, b.2, k),
        l.point(b.0, b.1, b.2, j),
    ]
}

/// A patch on a lathe's facet `f`: `u` across it from its first edge,
/// `v` along it from ring `a` to ring `b`, lifted `lift` off it, facing
/// away from `centre`.
#[allow(clippy::too_many_arguments)]
fn patch(
    s: &mut Shape,
    f: &[Vec3; 4],
    centre: Vec3,
    u0: f32,
    u1: f32,
    v0: f32,
    v1: f32,
    lift: f32,
) {
    let at = |u: f32, v: f32| f[0].lerp(f[1], u).lerp(f[3].lerp(f[2], u), v);
    let mut n = (f[1] - f[0]).cross(f[3] - f[0]).normalize_or_zero();
    let mid = (f[0] + f[1] + f[2] + f[3]) / 4.0;
    if n.dot(mid - centre) < 0.0 {
        n = -n;
    }
    let pts = [(u0, v0), (u1, v0), (u1, v1), (u0, v1)].map(|(u, v)| at(u, v) + n * lift);
    poly(s, &pts, n);
}

/// A thin plate from the chord `a_le`–`a_te` to `b_le`–`b_te`, `ha` /
/// `hb` thick across its span (winglets, doors, blades).
#[allow(clippy::too_many_arguments)]
fn plate(
    p: &mut Parts,
    slot: &'static str,
    a_le: Vec3,
    a_te: Vec3,
    b_le: Vec3,
    b_te: Vec3,
    ha: f32,
    hb: f32,
) {
    let span = (b_le + b_te - a_le - a_te) / 2.0;
    let chord = (a_te - a_le + b_te - b_le) / 2.0;
    let n = span.cross(chord).normalize_or(Vec3::X);
    let c = [
        a_le - n * ha / 2.0,
        b_le - n * hb / 2.0,
        b_te - n * hb / 2.0,
        a_te - n * ha / 2.0,
        a_le + n * ha / 2.0,
        b_le + n * hb / 2.0,
        b_te + n * hb / 2.0,
        a_te + n * ha / 2.0,
    ];
    hexa(p, all(slot), &c);
}

/// An airfoil (six points) of chord `zl`–`zt`, `th` thick, at (`x`, `y`).
fn airfoil(x: f32, y: f32, zl: f32, zt: f32, th: f32) -> [Vec3; 6] {
    let c = zt - zl;
    [
        (0.0, 0.0),
        (0.15, 0.55),
        (0.55, 0.42),
        (1.0, 0.0),
        (0.55, -0.35),
        (0.15, -0.38),
    ]
    .map(|(f, dy)| v3(x, y + dy * th, zl + f * c))
}

/// A wing panel with an airfoil section from `root` to `tip` ([x, y, zl,
/// zt, th] as [`wing`]).
fn wing_af(p: &mut Parts, slot: &'static str, root: [f32; 5], tip: [f32; 5]) {
    let a = airfoil(root[0], root[1], root[2], root[3], root[4]);
    let b = airfoil(tip[0], tip[1], tip[2], tip[3], tip[4]);
    let centre = (a.iter().chain(&b).copied().sum::<Vec3>()) / 12.0;
    for k in 0..6 {
        let n = (k + 1) % 6;
        let quad = [a[k], a[n], b[n], b[k]];
        let mid = quad.iter().copied().sum::<Vec3>() / 4.0;
        // Out from the chord line, not from the span's middle.
        let out = (mid - centre) * Vec3::new(0.0, 1.0, 1.0);
        poly(p.slot(slot), &quad, out);
    }
    let ca = a.iter().copied().sum::<Vec3>() / 6.0;
    let cb = b.iter().copied().sum::<Vec3>() / 6.0;
    poly(p.slot(slot), &a, ca - cb);
    poly(p.slot(slot), &b, cb - ca);
}

/// The wing's section at span `x` (≥ the root's): [y, zl, zt, th].
fn wing_at(w: &[[f32; 5]; 3], x: f32) -> [f32; 4] {
    let (a, b) = if x <= w[1][0] {
        (w[0], w[1])
    } else {
        (w[1], w[2])
    };
    let f = ((x - a[0]) / (b[0] - a[0])).clamp(0.0, 1.0);
    let l = |k: usize| a[k] + (b[k] - a[k]) * f;
    [l(1), l(2), l(3), l(4)]
}

/// A narrow-body twinjet.
pub(super) fn airliner(b: &mut Build, s: &AirlinerSpec) {
    let far = b.far();
    b.paint("body", s.body);
    b.paint("livery", s.livery);
    let (cy, r) = (s.cy, s.r);
    let sides_n = if far { 6 } else { 16 };
    let fuselage = Lathe::new(v3(0.0, cy, 0.0), Vec3::Z, Vec3::Y, sides_n);
    let np = s.nose_prof.len();
    let tp = s.tail_prof.len();
    let prof: Vec<(f32, f32, f32)> = if far {
        let n = s.nose_prof;
        let t = s.tail_prof;
        [n[0], n[4], n[np - 1], t[0], t[2], t[tp - 1]]
            .iter()
            .enumerate()
            .map(|(i, &(dz, rf, lf))| {
                let z = if i < 3 { s.nose + dz } else { s.tail + dz };
                (z, rf * r, lf * r)
            })
            .collect()
    } else {
        s.nose_prof
            .iter()
            .map(|&(dz, rf, lf)| (s.nose + dz, rf * r, lf * r))
            .chain(
                s.tail_prof
                    .iter()
                    .map(|&(dz, rf, lf)| (s.tail + dz, rf * r, lf * r)),
            )
            .collect()
    };
    fuselage.draw(b.root(), &prof, |_, _| "body", "body");
    let end = *prof.last().expect("a tail");
    let zc0 = s.nose + s.nose_prof[np - 1].0;
    let zc1 = s.tail + s.tail_prof[0].0;
    if !far {
        let p = b.root();
        let centre = |a: (f32, f32, f32), c: (f32, f32, f32)| {
            v3(0.0, cy + (a.2 + c.2) / 2.0, (a.0 + c.0) / 2.0)
        };
        // A patch on the left facet `j` (counted from the top, 16 round)
        // and its mirror on the right: `d` runs down the facet on both.
        let side_patch = |p: &mut Parts,
                          slot: &'static str,
                          a: (f32, f32, f32),
                          c: (f32, f32, f32),
                          j: u32,
                          d: (f32, f32),
                          v: (f32, f32),
                          lift: f32,
                          which: &[f32]| {
            for &x in which {
                let (jj, u0, u1) = if x < 0.0 {
                    (j, d.0, d.1)
                } else {
                    (15 - j, 1.0 - d.1, 1.0 - d.0)
                };
                let f = facet(&fuselage, a, c, jj);
                patch(p.slot(slot), &f, centre(a, c), u0, u1, v.0, v.1, lift);
            }
        };
        let both = [-1.0f32, 1.0];
        // The cockpit: two windscreen panels a side, then the side window.
        let ring = |i: usize| prof[i];
        side_patch(
            p,
            "glass",
            ring(4),
            ring(5),
            1,
            (0.12, 0.92),
            (0.2, 0.92),
            0.012,
            &both,
        );
        side_patch(
            p,
            "glass",
            ring(4),
            ring(5),
            2,
            (0.06, 0.8),
            (0.2, 0.92),
            0.012,
            &both,
        );
        side_patch(
            p,
            "glass",
            ring(5),
            ring(6),
            2,
            (0.1, 0.75),
            (0.05, 0.42),
            0.012,
            &both,
        );
        // The cabin: one band from zc0 to zc1.
        let (a, c) = (ring(np - 1), ring(np));
        let v = |z: f32| (z - zc0) / (zc1 - zc0);
        // A thin cheat line in the livery colour below the windows.
        side_patch(
            p,
            "livery",
            a,
            c,
            4,
            (0.16, 0.3),
            (0.01, 0.99),
            0.006,
            &both,
        );
        // Door outlines (facets 3 and 4: above and below the middle).
        let w = 0.05;
        let outline = |p: &mut Parts, z0: f32, z1: f32, rows: &[(u32, f32, f32)]| {
            let last = rows.len() - 1;
            for (k, &(j, d0, d1)) in rows.iter().enumerate() {
                side_patch(
                    p,
                    "paint_dark",
                    a,
                    c,
                    j,
                    (d0, d1),
                    (v(z0), v(z0 + w)),
                    0.01,
                    &both,
                );
                side_patch(
                    p,
                    "paint_dark",
                    a,
                    c,
                    j,
                    (d0, d1),
                    (v(z1 - w), v(z1)),
                    0.01,
                    &both,
                );
                if k == 0 {
                    side_patch(
                        p,
                        "paint_dark",
                        a,
                        c,
                        j,
                        (d0, d0 + 0.07),
                        (v(z0), v(z1)),
                        0.01,
                        &both,
                    );
                }
                if k == last {
                    side_patch(
                        p,
                        "paint_dark",
                        a,
                        c,
                        j,
                        (d1 - 0.07, d1),
                        (v(z0), v(z1)),
                        0.01,
                        &both,
                    );
                }
            }
        };
        let doors = [(3, 0.06, 1.0), (4, 0.0, 0.86)];
        for z in s.doors {
            outline(p, z, z + 0.85, &doors);
        }
        for &z in s.exits {
            outline(p, z, z + 0.5, &[(3, 0.2, 1.0)]);
        }
        for z in s.cargo {
            outline(p, z, z + 1.7, &[(5, 0.08, 0.9)]);
        }
        // The window row, between the doors and exits.
        let busy: Vec<(f32, f32)> = s
            .doors
            .iter()
            .map(|&z| (z, z + 0.85))
            .chain(s.exits.iter().map(|&z| (z, z + 0.5)))
            .collect();
        let mut z = s.doors[0] + 1.25;
        while z + 0.26 < s.doors[1] - 0.35 {
            let clear = busy
                .iter()
                .all(|&(z0, z1)| z + 0.26 < z0 - 0.15 || z > z1 + 0.15);
            if clear {
                side_patch(
                    p,
                    "glass",
                    a,
                    c,
                    3,
                    (0.3, 0.74),
                    (v(z), v(z + 0.26)),
                    0.01,
                    &both,
                );
            }
            z += 0.8;
        }
        // The belly fairing under the wing box.
        let bf = |z: f32, d: f32, w: f32| sec(z, cy - r - d, cy - r * 0.45, w);
        let [w0, _, _] = s.wing;
        loft(
            p.slot("body"),
            &[
                bf(w0[2] - 1.9, -0.05, r * 0.6),
                bf(w0[2] + 0.2, 0.16, r * 0.92),
                bf(w0[3] - 0.4, 0.16, r * 0.92),
                bf(w0[3] + 2.4, -0.08, r * 0.5),
            ],
            0.5,
            0.3,
            false,
        );
        // The APU's exhaust out of the tail cone.
        rod(
            p,
            "trim",
            v3(0.0, cy + end.2, end.0 - 0.05),
            v3(0.0, cy + end.2, end.0 + 0.25),
            end.1 * 0.55,
            8,
        );
    }
    // Wings, winglets, engines, the tailplane.
    let wg = s.wing.map(|[x, y, zl, zt, th]| [x, cy + y, zl, zt, th]);
    let tip = wg[2];
    sides(|x| {
        let p = b.root();
        let m = |w: [f32; 5]| [x * w[0], w[1], w[2], w[3], w[4]];
        if far {
            wing(p, all("metal"), m(wg[0]), m(wg[1]));
            wing(p, all("metal"), m(wg[1]), m(wg[2]));
        } else {
            wing_af(p, "metal", m(wg[0]), m(wg[1]));
            wing_af(p, "metal", m(wg[1]), m(wg[2]));
        }
        let t = |dx: f32, dy: f32, dzl: f32, dzt: f32| {
            (
                v3(x * (tip[0] + dx), tip[1] + dy, tip[2] + dzl),
                v3(x * (tip[0] + dx), tip[1] + dy, tip[3] + dzt),
            )
        };
        let (le0, te0) = t(0.0, 0.0, 0.0, 0.0);
        match s.winglet {
            Winglet::Sharklet => {
                let (le1, te1) = t(0.36, 0.48, 0.6, 0.15);
                let (le2, te2) = t(0.52, 2.35, 1.85, 0.55);
                if far {
                    plate(p, "livery", le0, te0, le2, te2, tip[4], 0.1);
                } else {
                    plate(p, "livery", le0, te0, le1, te1, tip[4], 0.1);
                    plate(p, "livery", le1, te1, le2, te2, 0.1, 0.05);
                }
            }
            Winglet::Split => {
                let (le1, te1) = t(0.3, 0.34, 0.35, 0.1);
                let (le2, te2) = t(0.4, 2.55, 1.95, 0.6);
                if far {
                    plate(p, "livery", le0, te0, le2, te2, tip[4], 0.1);
                } else {
                    plate(p, "livery", le0, te0, le1, te1, tip[4], 0.1);
                    plate(p, "livery", le1, te1, le2, te2, 0.1, 0.05);
                    let (le3, te3) = t(0.24, -0.95, 1.2, 0.4);
                    plate(p, "livery", le0, te0, le3, te3, 0.08, 0.04);
                }
            }
        }
        let [ex, ey, ez, er, flat] = s.eng;
        let eng = Lathe::new(
            v3(x * ex, ey, ez),
            Vec3::Z,
            Vec3::Y,
            if far { 6 } else { 14 },
        )
        .squash(flat, 1.0);
        if far {
            eng.draw(
                p,
                &[
                    (0.0, er * 0.9, 0.0),
                    (0.3, er, 0.0),
                    (3.0, er * 0.8, 0.0),
                    (4.6, 0.0, 0.0),
                ],
                |i, _| if i == 0 { "trim" } else { "livery" },
                "trim",
            );
        } else {
            // From the fan face out through the inlet, along the cowl,
            // the fan nozzle, the core and its plug.
            eng.draw(
                p,
                &[
                    (0.5, er * 0.84, 0.0),
                    (0.02, er * 0.88, 0.0),
                    (-0.03, er * 0.95, 0.0),
                    (0.2, er, 0.0),
                    (1.5, er * 1.0, 0.0),
                    (2.75, er * 0.9, 0.0),
                    (3.35, er * 0.74, 0.0),
                    (3.35, er * 0.56, 0.0),
                    (3.95, er * 0.5, 0.0),
                    (4.45, er * 0.33, 0.0),
                    (4.45, er * 0.2, 0.0),
                    (5.0, 0.0, 0.0),
                ],
                |i, _| match i {
                    0 => "paint_dark",
                    1 | 2 => "metal",
                    3..=5 => "livery",
                    6 => "trim",
                    7 | 8 => "metal",
                    9 => "trim",
                    _ => "metal",
                },
                "trim",
            );
            // The spinner and the fan's blades.
            let sp = Lathe::new(v3(x * ex, ey, ez), Vec3::Z, Vec3::Y, 10).squash(flat, 1.0);
            sp.draw(
                p,
                &[
                    (0.08, 0.0, 0.0),
                    (0.3, er * 0.15, 0.0),
                    (0.49, er * 0.25, 0.0),
                ],
                |_, _| "metal",
                "metal",
            );
            let blades = 16;
            for k in 0..blades {
                let a = k as f32 * TAU / blades as f32;
                let dir = v3(a.sin() * x, a.cos() * flat, 0.0);
                let across = v3(a.cos() * x, -a.sin() * flat, 0.0);
                let c0 = v3(x * ex, ey, ez + 0.47);
                let (r0, r1) = (er * 0.25, er * 0.82);
                let pts = [
                    c0 + dir * r0 - across * 0.05,
                    c0 + dir * r1 - across * 0.09 + Vec3::Z * 0.06,
                    c0 + dir * r1 + across * 0.09 - Vec3::Z * 0.06,
                    c0 + dir * r0 + across * 0.05,
                ];
                poly(p.slot("paint_dark"), &pts, -Vec3::Z);
            }
            // The pylon: from the nacelle's top up into the wing.
            let [wy, wzl, _, wth] = wing_at(&wg, ex);
            let top = ey + er * flat * 0.9;
            let under = wy - wth * 0.3;
            let c = slab(
                ez + 1.0,
                ez + 3.5,
                top,
                0.14,
                ez + 0.6,
                wzl + 2.6,
                under,
                0.11,
            )
            .map(|k| k + Vec3::X * x * ex);
            hexa(p, all("body"), &c);
            // Flap-track fairings under the trailing edge.
            for fx in [3.4, 8.6, 11.6, 14.4] {
                let fx = fx * (tip[0] / 16.6);
                let [wy, _, wzt, wth] = wing_at(&wg, fx);
                let y0 = wy - wth * 0.3;
                let c = [
                    v3(-0.09, y0 - 0.32, wzt - 1.1),
                    v3(0.09, y0 - 0.32, wzt - 1.1),
                    v3(0.05, y0 - 0.12, wzt + 0.95),
                    v3(-0.05, y0 - 0.12, wzt + 0.95),
                    v3(-0.12, y0 + 0.05, wzt - 1.7),
                    v3(0.12, y0 + 0.05, wzt - 1.7),
                    v3(0.04, y0 + 0.05, wzt + 0.95),
                    v3(-0.04, y0 + 0.05, wzt + 0.95),
                ]
                .map(|k| k + Vec3::X * x * fx);
                hexa(p, all("body"), &c);
            }
        }
        let tw = s
            .tailplane
            .map(|[tx, ty, zl, zt, th]| [x * tx, cy + ty, zl, zt, th]);
        if far {
            wing(p, all("metal"), tw[0], tw[1]);
        } else {
            wing_af(p, "metal", tw[0], tw[1]);
        }
    });
    let [fy0, fl0, ft0, fy1, fl1, ft1, dorsal] = s.fin;
    fin(
        b.root(),
        "livery",
        0.0,
        cy + fy0,
        fl0,
        ft0,
        0.4,
        cy + fy1,
        fl1,
        ft1,
        0.16,
    );
    if !far {
        // The dorsal fillet, from the fuselage's top into the fin.
        let lf = fl0 + (fl1 - fl0) * (2.75 - fy0) / (fy1 - fy0);
        fin(
            b.root(),
            "livery",
            0.0,
            cy + r * 0.9,
            dorsal,
            fl0 + 0.4,
            0.3,
            cy + 2.75,
            lf - 0.1,
            lf + 0.3,
            0.2,
        );
        gear(b, s);
    }
    let nav_x = tip[0]
        + if s.winglet == Winglet::Sharklet {
            0.4
        } else {
            0.34
        };
    let nav_y = tip[1] + 0.42;
    nav_lights(
        b,
        v3(-nav_x, nav_y, tip[2] + 0.75),
        v3(nav_x, nav_y, tip[2] + 0.75),
        v3(0.0, cy + end.2 + end.1 + 0.08, end.0 + 0.05),
        v3(0.0, cy + r + 0.1, -2.0),
        0.1,
    );
    sides(|x| {
        b.lamp(
            &format!("light_strobe_{}", lr(x)),
            ROOT,
            "light_white",
            v3(x * nav_x, nav_y, tip[3] + 0.3),
            Vec3::splat(0.08),
            Vec3::Z,
        );
    });
}

/// The landing gear, down: the nose leg with its twin wheels and bay
/// doors, the main legs with their side braces, twin wheels and leg
/// doors.
fn gear(b: &mut Build, s: &AirlinerSpec) {
    let (cy, r) = (s.cy, s.r);
    let [nz, nr, mx, mz, mr] = s.gear;
    let belly = cy - r * 0.97;
    let at = v3(0.0, nr, nz);
    let p = b.part("gear_nose", ROOT, at);
    rod(p, "metal", at, v3(0.0, belly + 0.2, nz - 0.1), 0.09, 6);
    rod(
        p,
        "metal",
        v3(0.0, nr + 0.3, nz),
        v3(0.0, belly - 0.2, nz + 0.75),
        0.05,
        6,
    );
    sides(|x| wheel(p, v3(x * 0.2, nr, nz), x, nr, 0.22, 10, true));
    sides(|x| {
        cuboid(
            p,
            all("body"),
            v3(x * 0.44, belly - 0.4, nz + 0.1),
            v3(0.025, 0.42, 0.75),
        );
    });
    sides(|x| {
        let name = format!("gear_main_{}", lr(x));
        let at = v3(x * mx, mr, mz);
        let p = b.part(&name, ROOT, at);
        let [wy, _, _, wth] = wing_at(&s.wing.map(|[a, y, c, d, e]| [a, cy + y, c, d, e]), mx);
        let top = wy - wth * 0.3;
        rod(p, "metal", at, v3(x * mx, top, mz), 0.13, 6);
        rod(
            p,
            "metal",
            v3(x * mx, mr + 1.0, mz),
            v3(x * (r * 0.62), cy - r * 0.8, mz - 0.4),
            0.06,
            6,
        );
        for dx in [-0.3f32, 0.3] {
            let side = if dx * x > 0.0 { x } else { -x };
            wheel(p, at + Vec3::X * dx, side, mr, 0.34, 10, true);
        }
        // The leg door, outboard of the leg.
        cuboid(
            p,
            all("body"),
            v3(x * (mx + 0.14), (mr * 2.0 + 0.15 + top) / 2.0, mz - 0.05),
            v3(0.025, (top - mr * 2.0 - 0.15) / 2.0, 0.5),
        );
    });
}

/// The medium transport helicopter (`ref-3`'s proportions): a big glazed
/// nose, a boxy cabin with round windows and a sliding door, two engine
/// cowlings over it with the gearbox, a five-blade main rotor drooping at
/// rest, the tail boom with its swept pylon and a three-blade tail rotor,
/// fuel tanks and outrigger pylons on the sides, fixed tricycle gear; a
/// sand tan with a darker band (our own).
pub(super) fn helicopter_medium(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xC8B48C);
    b.paint("livery", 0x7D6A4B);
    let p = b.root();
    // The cabin: an octagonal loft from behind the nose to the boom.
    let secs = [
        sec(-6.9, 0.78, 3.0, 1.2),
        sec(-5.8, 0.72, 3.15, 1.26),
        sec(2.4, 0.72, 3.15, 1.26),
        sec(3.6, 0.95, 3.1, 1.1),
        sec(5.2, 1.95, 3.0, 0.62),
    ];
    if far {
        loft(p.slot("body"), &[secs[0], secs[2], secs[4]], 0.3, 0.4, true);
    } else {
        loft(p.slot("body"), &secs, 0.32, 0.42, false);
    }
    // The nose: a lathe, glazed but for its chin and the frames' band.
    let n = if far { 6 } else { 12 };
    let nose = Lathe::new(v3(0.0, 1.95, 0.0), Vec3::Z, Vec3::Y, n).squash(1.05, 1.0);
    let prof: Vec<(f32, f32, f32)> = if far {
        vec![(-8.9, 0.0, -0.1), (-7.9, 1.0, 0.0), (-6.4, 1.15, 0.0)]
    } else {
        vec![
            (-8.95, 0.0, -0.12),
            (-8.85, 0.42, -0.1),
            (-8.5, 0.8, -0.05),
            (-7.95, 1.04, 0.0),
            (-7.3, 1.15, 0.0),
            (-6.4, 1.16, 0.0),
        ]
    };
    nose.draw(
        b.root(),
        &prof,
        |i, j| {
            let bottom = (4..=8).contains(&j);
            if far {
                if i == 1 && !bottom { "glass" } else { "body" }
            } else if (i == 1 || i == 3) && !bottom
                || i == 2 && matches!(j, 5 | 7)
                || i == 4 && matches!(j, 2 | 10)
            {
                "glass"
            } else {
                "body"
            }
        },
        "body",
    );
    let p = b.root();
    if !far {
        let cab = slab(-6.9, 5.2, 0.72, 1.26, -6.9, 5.2, 3.15, 1.26);
        // `u` from the front on both sides (a left face runs from the back).
        let uu = |face: usize, u0: f32, u1: f32| {
            if face == LEFT {
                (1.0 - u1, 1.0 - u0)
            } else {
                (u0, u1)
            }
        };
        sides(|x| {
            let face = if x < 0.0 { LEFT } else { RIGHT };
            // The lower band in the darker tone.
            let (a, c) = uu(face, 0.1, 0.75);
            decal(p.slot("livery"), &cab, face, a, c, 0.15, 0.27, 0.01);
            // Round-ish cabin windows.
            for k in 0..5 {
                let (a, c) = uu(face, 0.16 + k as f32 * 0.085, 0.2 + k as f32 * 0.085);
                decal(p.slot("glass"), &cab, face, a, c, 0.6, 0.76, 0.012);
            }
        });
        // The sliding door's outline on the left, the cockpit doors.
        let mut frame = |face: usize, u0: f32, u1: f32, v0: f32, v1: f32| {
            let (u0, u1) = uu(face, u0, u1);
            for (a, bb, c, d) in [
                (u0, u1, v0, v0 + 0.02),
                (u0, u1, v1 - 0.02, v1),
                (u0, u0 + 0.006, v0, v1),
                (u1 - 0.006, u1, v0, v1),
            ] {
                decal(p.slot("paint_dark"), &cab, face, a, bb, c, d, 0.014);
            }
        };
        frame(LEFT, 0.11, 0.21, 0.14, 0.82);
        frame(LEFT, 0.02, 0.08, 0.3, 0.82);
        frame(RIGHT, 0.02, 0.08, 0.3, 0.82);
    }
    // The engines' cowlings over the cabin and the gearbox behind them.
    sides(|x| {
        let p = b.root();
        let e = Lathe::new(
            v3(x * 0.56, 3.48, 0.0),
            Vec3::Z,
            Vec3::Y,
            if far { 5 } else { 10 },
        )
        .squash(1.0, 1.05);
        if far {
            if x < 0.0 {
                hexa(
                    p,
                    all("body"),
                    &slab(-5.0, 0.2, 3.1, 1.0, -4.8, 0.0, 3.92, 0.95),
                );
            }
        } else {
            e.draw(
                p,
                &[
                    (-5.0, 0.34, 0.0),
                    (-4.85, 0.42, 0.0),
                    (-3.6, 0.45, 0.0),
                    (-0.6, 0.45, 0.0),
                    (0.2, 0.3, 0.0),
                    (0.3, 0.0, 0.0),
                ],
                |i, _| if i == 0 { "metal" } else { "body" },
                "trim",
            );
            // The exhausts, turned out to the sides.
            rod(
                p,
                "trim",
                v3(x * 0.72, 3.45, -0.2),
                v3(x * 1.38, 3.3, 0.5),
                0.22,
                8,
            );
        }
    });
    let p = b.root();
    hexa(
        p,
        all("body"),
        &slab(-0.9, 2.2, 3.1, 0.62, -0.5, 1.9, 3.95, 0.45),
    );
    if !far {
        // The oil cooler's hood in front of the gearbox.
        hexa(
            p,
            all("body"),
            &slab(-1.5, -0.6, 3.6, 0.42, -1.3, -0.7, 4.15, 0.34),
        );
        decal(
            p.slot("trim"),
            &slab(-1.5, -0.6, 3.6, 0.42, -1.3, -0.7, 4.15, 0.34),
            FRONT,
            0.15,
            0.85,
            0.2,
            0.85,
            0.01,
        );
        rod(p, "metal", v3(0.0, 3.9, 0.6), v3(0.0, 4.42, 0.6), 0.16, 8);
    }
    // The tail boom, its pylon and the tailplane.
    let boom = Lathe::new(
        v3(0.0, 2.55, 0.0),
        Vec3::Z,
        Vec3::Y,
        if far { 5 } else { 10 },
    );
    boom.draw(
        p,
        &[(4.6, 0.62, 0.0), (8.0, 0.42, 0.35), (11.2, 0.3, 0.8)],
        |_, _| "body",
        "body",
    );
    fin(p, "body", 0.0, 3.0, 10.4, 11.7, 0.36, 5.0, 11.4, 12.1, 0.22);
    wing(
        p,
        all("body"),
        [-1.6, 2.95, 9.6, 10.5, 0.1],
        [1.6, 2.95, 9.7, 10.5, 0.08],
    );
    if !far {
        // A band in the darker tone round the boom, the tail skid.
        fin(
            p, "livery", 0.0, 4.4, 11.13, 11.98, 0.3, 5.0, 11.4, 12.1, 0.24,
        );
        rod(
            p,
            "metal",
            v3(0.0, 2.85, 10.6),
            v3(0.0, 1.95, 11.2),
            0.05,
            6,
        );
    }
    // Fuel tanks low on the sides, the outrigger pylons above them.
    sides(|x| {
        let p = b.root();
        let t = Lathe::new(
            v3(x * 1.55, 1.25, 0.0),
            Vec3::Z,
            Vec3::Y,
            if far { 5 } else { 10 },
        )
        .squash(1.0, 0.85);
        if far {
            cuboid(p, all("body"), v3(x * 1.55, 1.25, -1.4), v3(0.42, 0.5, 2.4));
        } else {
            t.draw(
                p,
                &[
                    (-3.9, 0.0, 0.0),
                    (-3.65, 0.3, 0.0),
                    (-3.1, 0.5, 0.0),
                    (0.3, 0.5, 0.0),
                    (0.8, 0.36, 0.0),
                    (1.05, 0.0, 0.0),
                ],
                |i, _| if i == 3 { "livery" } else { "body" },
                "body",
            );
            // The outrigger: a beam with two hard points and their pods.
            cuboid(
                p,
                all("metal"),
                v3(x * 2.25, 1.89, -3.0),
                v3(0.95, 0.07, 0.36),
            );
            for dx in [1.95f32, 2.85] {
                cuboid(p, all("trim"), v3(x * dx, 1.62, -3.0), v3(0.12, 0.2, 0.45));
            }
            // The tank's struts.
            rod(
                p,
                "metal",
                v3(x * 1.55, 1.7, -1.2),
                v3(x * 1.2, 2.0, -1.2),
                0.04,
                6,
            );
        }
    });
    // Fixed tricycle gear: twin nose wheels, a main wheel a side on its
    // struts.
    if !far {
        let p = b.root();
        rod(
            p,
            "metal",
            v3(0.0, 0.34, -6.3),
            v3(0.0, 0.95, -6.2),
            0.07,
            6,
        );
        sides(|x| wheel(p, v3(x * 0.2, 0.34, -6.3), x, 0.34, 0.18, 10, true));
        sides(|x| {
            let at = v3(x * 2.05, 0.5, 1.7);
            wheel(p, at, x, 0.5, 0.28, 10, true);
            rod(p, "metal", at, v3(x * 1.15, 0.95, 1.1), 0.07, 6);
            rod(p, "metal", at, v3(x * 1.1, 1.0, 2.4), 0.06, 6);
            rod(
                p,
                "metal",
                at + Vec3::Y * 0.2,
                v3(x * 1.45, 1.75, 1.35),
                0.09,
                6,
            );
        });
    } else {
        let p = b.root();
        sides(|x| cuboid(p, all("tyre"), v3(x * 2.05, 0.5, 1.7), v3(0.14, 0.5, 0.5)));
        cuboid(p, all("tyre"), v3(0.0, 0.34, -6.3), v3(0.3, 0.34, 0.34));
    }
    // The main rotor: five blades about Y, drooping at rest.
    let hub = v3(0.0, 4.5, 0.6);
    let p = b.part("rotor_main", ROOT, hub);
    if !far {
        let h = Lathe::new(hub - Vec3::Y * 0.12, Vec3::Y, Vec3::Z, 10);
        h.draw(
            p,
            &[
                (0.0, 0.0, 0.0),
                (0.0, 0.5, 0.0),
                (0.24, 0.42, 0.0),
                (0.34, 0.0, 0.0),
            ],
            |_, _| "metal",
            "metal",
        );
    }
    let blades = 5;
    for k in 0..blades {
        let a = k as f32 * TAU / blades as f32 + 0.3;
        let rot = Quat::from_rotation_y(a) * Quat::from_rotation_x(-0.075);
        obox(
            p,
            all("trim"),
            hub + rot * v3(0.0, 0.0, -4.95),
            v3(0.27, 0.035, 4.45),
            rot,
        );
        if !far {
            // The hinge and the blade's root sleeve.
            obox(
                p,
                all("metal"),
                hub + rot * v3(0.0, 0.0, -0.55),
                v3(0.12, 0.08, 0.28),
                rot,
            );
        }
    }
    // The tail rotor on the pylon's left, three blades about X.
    let th = v3(-0.32, 4.55, 11.9);
    let p = b.part("rotor_tail", ROOT, th);
    if far {
        cuboid(p, all("trim"), th, v3(0.02, 1.54, 0.11));
    }
    for k in 0..if far { 0 } else { 3 } {
        let a = k as f32 * TAU / 3.0;
        let rot = Quat::from_rotation_x(a);
        obox(
            p,
            all("trim"),
            th + rot * v3(0.0, 0.82, 0.0),
            v3(0.02, 0.72, 0.11),
            rot,
        );
    }
    if !far {
        rod(
            p,
            "metal",
            th - Vec3::X * 0.12,
            th + Vec3::X * 0.25,
            0.08,
            6,
        );
    }
    nav_lights(
        b,
        v3(-3.3, 1.9, -3.0),
        v3(3.3, 1.9, -3.0),
        v3(0.0, 3.05, 11.78),
        v3(0.0, 3.98, 1.6),
        0.07,
    );
}
