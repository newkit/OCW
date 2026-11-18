// Roads (spec §4.6 step 6, Phase 8 T2 / T5): the standard PBR fragment
// shader with the road material of each part, the markings and the wear.
//
// Vertex data (crate::road_mesh): uv = (across, along) in metres (along
// the road edge's arc length); uv_b.x = layer + 8 * kind + 64 * extra
// (kind 0 carriageway, 1 wall, 2 shoulder, 3 kerb, 4 sidewalk, 5 crossing
// overlay; extra: cycle lanes, track); uv_b.y = the lines code
// (ocw_world::road_dress::MarkCode) or, on an overlay, its crossing kind +
// 4 * half; colour = far colour (rgb) and the road's half width (alpha).
//
// The markings are coverage integrated over each pixel's footprint in road
// metres (box filter, Rust twins in ocw_world::road_dress): crisp near, the
// mean grey far away, no shimmer. Independent of the light setup.
//
// Damage (ocw_world::road_wear, the placement's Rust twin): by the strip's
// wear grade (bits 5-7 of extra; from OSM smoothness, else estimated) -
// transverse and longitudinal cracks, sealed cracks, alligator areas,
// repaired patches, wheel tracks, worn paint, loose stones, ruts on
// unpaved roads, concrete joints (bit 8), and potholes recessed by a
// parallax step and their normals (the road's geometry is not changed),
// holding water when the road is wet; main roads (bit 9) keep their
// potholes near the edges.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::view,
};

struct RoadLook {
    // x: textured; y, z: fade start, end (m); w: normal strength.
    mode: vec4<f32>,
    sizes: array<vec4<f32>, 2>,
    // width, dash, gap; w: double-line gap.
    centre: vec4<f32>,
    // width, dash, gap; w: edge line inset.
    centre_town: vec4<f32>,
    // width, dash, gap; w: edge line width.
    lane: vec4<f32>,
    // width, dash, gap; w: cycle lane width.
    lane_town: vec4<f32>,
    // advisory width, dash, gap; w: exclusive width.
    cycle: vec4<f32>,
    // bar, gap, length; w: stop line width.
    zebra: vec4<f32>,
    // width, dash, gap; w: signals spacing.
    give_way: vec4<f32>,
    // width, dash, gap; w: stop line distance.
    signals: vec4<f32>,
    // rgb; w: wear strength.
    line_colour: vec4<f32>,
    // rgb; w: 1 = an untyped cycle lane is exclusive.
    cycle_colour: vec4<f32>,
    // rgb; w: kerb stones' top width (m).
    crossing_colour: vec4<f32>,
    // The wear per grade (ocw_world::road_wear::wear_density; index =
    // grade, 0 draws nothing): x transverse, y longitudinal crack chance,
    // z alligator area chance, w pothole chance.
    damage: array<vec4<f32>, 8>,
    // x patch chance, y sealed share of the cracks, z pothole zone from the
    // edge (m), w wheel track strength.
    repair: array<vec4<f32>, 8>,
    // x loose stone chance per 10 cm cell, y rut depth of unpaved roads.
    loose: array<vec4<f32>, 8>,
    // The quality preset (crate::road_material::damage_detail): x 1 = the
    // alligator crack pattern is drawn; y damage drawn up to (m); z the
    // road.damage override (0 auto, 1 … 7 a grade, 8 none).
    detail: vec4<f32>,
    // The weather: x wetness, y puddles, z snow cover, w wind sway.
    weather: vec4<f32>,
    // The season (spec §14 Phase 9, S8): x, y the climatological snow
    // ramp over the altitude; z on.
    season: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> look: RoadLook;
@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var albedo_array: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102)
var array_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103)
var normal_array: texture_2d_array<f32>;

// Share of the pixel [u - du/2, u + du/2] inside [a, b].
fn band_cov(u: f32, du: f32, a: f32, b: f32) -> f32 {
    return clamp((min(u + 0.5 * du, b) - max(u - 0.5 * du, a)) / du, 0.0, 1.0);
}

// Painted length of a dash pattern (dash, period p) from 0 to x.
fn dash_int(x: f32, dash: f32, p: f32) -> f32 {
    let k = floor(x / p);
    return k * dash + clamp(x - k * p, 0.0, dash);
}

// Share of the pixel along the road that a dashed pattern paints.
fn dash_cov(v: f32, dv: f32, dash: f32, gap: f32) -> f32 {
    if gap <= 0.0 {
        return 1.0;
    }
    let p = dash + gap;
    if dv >= p {
        return dash / p;
    }
    // The phase first: v itself can be kilometres.
    let x = v - floor(v / p) * p;
    return clamp((dash_int(x + 0.5 * dv, dash, p) - dash_int(x - 0.5 * dv, dash, p)) / dv, 0.0, 1.0);
}

// A line (width, dash, gap) centred at `at` across the road.
fn line_cov(u: f32, du: f32, v: f32, dv: f32, at: f32, s: vec3<f32>) -> f32 {
    return band_cov(u, du, at - 0.5 * s.x, at + 0.5 * s.x) * dash_cov(v, dv, s.y, s.z);
}

// The longitudinal markings (ocw_world::road_dress::marking_lines): (line
// paint, cycle lane paint).
fn markings(u: f32, du: f32, v: f32, dv: f32, half: f32, lines: u32, extra: u32) -> vec2<f32> {
    let lf = lines & 7u;
    let lb = (lines >> 3u) & 7u;
    let centre = (lines >> 6u) & 7u;
    let edges = ((lines >> 9u) & 1u) == 1u;
    let lane_lines = ((lines >> 10u) & 1u) == 1u;
    let town = ((lines >> 11u) & 1u) == 1u;
    var paint = 0.0;
    var cycle = 0.0;
    var lo = -half;
    var hi = half;
    let cw = look.lane_town.w;
    for (var side = 0u; side < 2u; side++) {
        var c = (extra >> (2u * side)) & 3u;
        if c == 1u {
            c = select(2u, 3u, look.cycle_colour.w > 0.5);
        }
        let sgn = select(-1.0, 1.0, side == 1u);
        if c >= 2u && half > cw + 1.0 {
            let at = sgn * (half - cw);
            var s = look.cycle.xyz;
            if c == 3u {
                s = vec3<f32>(look.cycle.w, 1.0, 0.0);
            }
            cycle = max(cycle, line_cov(u, du, v, dv, at, s));
            if side == 0u {
                lo = at;
            } else {
                hi = at;
            }
        } else if edges && !town {
            let at = sgn * (half - look.centre_town.w - 0.5 * look.lane.w);
            paint = max(paint, line_cov(u, du, v, dv, at, vec3<f32>(look.lane.w, 1.0, 0.0)));
        }
    }
    var cs = look.centre.xyz;
    var ls = look.lane.xyz;
    if town {
        cs = look.centre_town.xyz;
        ls = look.lane_town.xyz;
    }
    var c = lo;
    if lf > 0u && lb > 0u {
        c = lo + (hi - lo) * f32(lb) / f32(lf + lb);
    }
    let g = 0.5 * look.centre.w;
    let solid = vec3<f32>(cs.x, 1.0, 0.0);
    switch centre {
        case 1u: {
            paint = max(paint, line_cov(u, du, v, dv, c, cs));
        }
        case 2u: {
            paint = max(paint, line_cov(u, du, v, dv, c, solid));
        }
        case 3u: {
            paint = max(paint, max(line_cov(u, du, v, dv, c - g, solid), line_cov(u, du, v, dv, c + g, solid)));
        }
        case 4u: {
            paint = max(paint, max(line_cov(u, du, v, dv, c - g, cs), line_cov(u, du, v, dv, c + g, solid)));
        }
        case 5u: {
            paint = max(paint, max(line_cov(u, du, v, dv, c - g, solid), line_cov(u, du, v, dv, c + g, cs)));
        }
        default: {}
    }
    if lane_lines {
        var a0 = lo;
        var z0 = hi;
        var n0 = lf;
        var a1 = 0.0;
        var z1 = 0.0;
        var n1 = 0u;
        if lf == 0u {
            n0 = lb;
        } else if lb > 0u {
            z0 = c;
            n0 = lb;
            a1 = c;
            z1 = hi;
            n1 = lf;
        }
        for (var k = 1u; k < n0; k++) {
            paint = max(paint, line_cov(u, du, v, dv, a0 + (z0 - a0) * f32(k) / f32(n0), ls));
        }
        for (var k = 1u; k < n1; k++) {
            paint = max(paint, line_cov(u, du, v, dv, a1 + (z1 - a1) * f32(k) / f32(n1), ls));
        }
    }
    return vec2<f32>(paint, cycle);
}

// A transverse marking (ocw_world::road_dress::crossing_paint_at): (line
// paint, crossing paint) at (u, t along the overlay).
fn crossing(u: f32, du: f32, t: f32, dt: f32, half: f32, kind: u32, h: u32) -> vec2<f32> {
    var in_half = band_cov(u, du, -half, half);
    if h == 1u {
        in_half = band_cov(u, du, 0.0, half);
    } else if h == 2u {
        in_half = band_cov(u, du, -half, 0.0);
    }
    let stop = look.zebra.w;
    switch kind {
        case 0u: {
            let bar = look.zebra.x;
            let p = bar + look.zebra.y;
            let k = band_cov(u, du, 0.2 - half, half - 0.2) * band_cov(t, dt, 0.0, look.zebra.z)
                * dash_cov(u + 0.5 * bar + p * 64.0, du, bar, look.zebra.y);
            return vec2<f32>(0.0, k);
        }
        case 1u: {
            let s = look.signals.w + stop;
            let w = look.signals.x;
            let sp = look.give_way.w;
            let lines = max(band_cov(t, dt, s, s + w), band_cov(t, dt, s + sp, s + sp + w))
                * band_cov(u, du, -half, half) * dash_cov(u + half, du, look.signals.y, look.signals.z);
            let end = s + sp + w + look.signals.w;
            let f = band_cov(t, dt, 0.0, stop) * band_cov(u, du, 0.0, half);
            let b = band_cov(t, dt, end, end + stop) * band_cov(u, du, -half, 0.0);
            return vec2<f32>(max(lines, max(f, b)), 0.0);
        }
        case 2u: {
            return vec2<f32>(band_cov(t, dt, 0.0, stop) * in_half, 0.0);
        }
        case 3u: {
            let gw = look.give_way;
            return vec2<f32>(band_cov(t, dt, 0.0, gw.x) * in_half * dash_cov(u + half, du, gw.y, gw.z), 0.0);
        }
        default: {
            return vec2<f32>(0.0);
        }
    }
}

// Integer hash (ocw_world::road_dress::hash2) and its unit value.
fn hash2(a: i32, b: i32) -> u32 {
    var h = bitcast<u32>(a) * 0x8da6b343u + bitcast<u32>(b) * 0xd8163841u + 0x9e3779b9u;
    h = h ^ (h >> 15u);
    h = h * 0x2c1b3c6du;
    h = h ^ (h >> 12u);
    h = h * 0x297a2d39u;
    h = h ^ (h >> 15u);
    return h;
}

fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let s = f * f * (3.0 - 2.0 * f);
    let x = i32(i.x);
    let y = i32(i.y);
    let a = unit(hash2(x, y));
    let b = unit(hash2(x + 1, y));
    let c = unit(hash2(x, y + 1));
    let d = unit(hash2(x + 1, y + 1));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// A repaired patch (ocw_world::road_dress::patch_at), filtered.
fn patch_cov(half: f32, u: f32, du: f32, v: f32, dv: f32, chance: f32) -> f32 {
    let k = i32(floor(v / 8.0));
    let side = select(0, 1, u >= 0.0);
    if unit(hash2(k, side)) >= chance {
        return 0.0;
    }
    let start = f32(k) * 8.0 + unit(hash2(k, side + 7)) * 3.0;
    let len = 1.5 + unit(hash2(k, side + 13)) * 4.0;
    let w = half * (0.45 + 0.5 * unit(hash2(k, side + 29)));
    var across = band_cov(u, du, 0.0, w);
    if side == 0 {
        across = band_cov(u, du, -w, 0.0);
    }
    return across * band_cov(v, dv, start, start + len);
}

// A transverse crack (x) or its sealing line (y), filtered; a fine crack
// fades when the pixel is wider than a few centimetres.
fn transverse(u: f32, du: f32, v: f32, dv: f32, half: f32, chance: f32, sealed: f32) -> vec2<f32> {
    let k = i32(floor(v / 11.0));
    let h = hash2(k, 101);
    if unit(h) >= chance {
        return vec2<f32>(0.0);
    }
    let v0 = (f32(k) + 0.15 + 0.7 * unit(hash2(k, 103))) * 11.0
        + 0.15 * sin(u * 1.7 + unit(h) * 40.0) + 0.04 * sin(u * 6.1);
    let reach = half * (0.4 + 0.6 * unit(hash2(k, 107)));
    let across = band_cov(u, du, -reach, reach);
    let fp = max(dv, 1e-3);
    if unit(hash2(k, 109)) < sealed {
        return vec2<f32>(0.0, band_cov(v, fp, v0 - 0.022, v0 + 0.022) * across);
    }
    let fade = 1.0 - smoothstep(0.03, 0.12, dv);
    return vec2<f32>(band_cov(v, fp, v0 - 0.006, v0 + 0.006) * across * fade, 0.0);
}

// The longitudinal cracks (ocw_world::road_wear::longitudinal_in_cell):
// beside the middle joint and in the edge zones; (crack, sealing line).
fn longitudinal(u: f32, du: f32, v: f32, dv: f32, half: f32, chance: f32, sealed: f32) -> vec2<f32> {
    let k = i32(floor(v / 16.0));
    var out = vec2<f32>(0.0);
    for (var slot = 0; slot < 3; slot++) {
        if unit(hash2(k, 401 + slot)) >= chance {
            continue;
        }
        let start = f32(k) * 16.0 + 0.5 + 6.0 * unit(hash2(k, 409 + slot));
        let len = 2.0 + 7.0 * unit(hash2(k, 419 + slot));
        let along = band_cov(v, dv, start, start + len);
        if along <= 0.0 {
            continue;
        }
        let o = unit(hash2(k, 421 + slot));
        var uc = 0.08 + 0.12 * o;
        if slot == 1 {
            uc = -(half - 0.3 - 0.5 * o);
        } else if slot == 2 {
            uc = half - 0.3 - 0.5 * o;
        }
        let ph = o * 37.0;
        uc = uc + 0.06 * sin(v * 0.9 + ph) + 0.02 * sin(v * 3.7 + ph * 1.3);
        let fp = max(du, 1e-3);
        if unit(hash2(k, 431 + slot)) < sealed {
            out.y = max(out.y, band_cov(u, fp, uc - 0.022, uc + 0.022) * along);
        } else {
            let fade = 1.0 - smoothstep(0.03, 0.12, du);
            out.x = max(out.x, band_cov(u, fp, uc - 0.006, uc + 0.006) * along * fade);
        }
    }
    return out;
}

// Alligator (fatigue) cracking (ocw_world::road_wear::alligator_in_cell):
// (area, crack lines). The lines are the edges of a Voronoi pattern of
// ~22 cm cells, only drawn when `pattern` (the quality preset).
fn alligator(u: f32, du: f32, v: f32, dv: f32, half: f32, chance: f32, track: bool, pattern: bool) -> vec2<f32> {
    let k = i32(floor(v / 20.0));
    var area = 0.0;
    for (var side = 0; side < 2; side++) {
        if unit(hash2(k, 307 + side)) >= chance {
            continue;
        }
        let start = (f32(k) + 0.1 + 0.5 * unit(hash2(k, 311 + side))) * 20.0;
        let len = 2.0 + 5.0 * unit(hash2(k, 313 + side));
        var at = max(half - 0.9, 0.3);
        if track {
            at = 0.8;
        }
        let uc = select(-1.0, 1.0, side == 1) * at;
        let hw = min(0.25 + 0.35 * unit(hash2(k, 317 + side)), half * 0.5);
        let a = (u - uc) / hw;
        let b = (v - start - 0.5 * len) / (0.5 * len);
        let e = a * a + b * b + 0.6 * (vnoise(vec2<f32>(u, v) * 2.5 + f32(side) * 13.0) - 0.5);
        area = max(area, 1.0 - smoothstep(0.7, 1.0, e));
    }
    if area <= 0.0 || !pattern {
        return vec2<f32>(area, 0.0);
    }
    let p = vec2<f32>(u, v) / 0.22;
    let ip = floor(p);
    let fp = p - ip;
    var f1 = 8.0;
    var f2 = 8.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let x = i32(ip.x) + i;
            let y = i32(ip.y) + j;
            let o = vec2<f32>(unit(hash2(x, y * 7 + 501)), unit(hash2(x * 13 + 3, y + 509)));
            let d = length(vec2<f32>(f32(i), f32(j)) + 0.15 + 0.7 * o - fp);
            if d < f1 {
                f2 = f1;
                f1 = d;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    let edge = 0.5 * (f2 - f1) * 0.22;
    let fpm = max(max(du, dv), 1e-3);
    let fade = 1.0 - smoothstep(0.04, 0.12, fpm);
    return vec2<f32>(area, band_cov(edge, fpm, -0.007, 0.007) * fade * area);
}

// A loose stone (ocw_world::road_wear::stone_in_cell) in its 10 cm cell:
// (cover, shade 0 … 1, the tilt of its dome's normal across and along).
fn stone(u: f32, v: f32, fp: f32, chance: f32) -> vec4<f32> {
    let c = 0.1;
    let i = i32(floor(u / c));
    let j = i32(floor(v / c));
    if unit(hash2(i * 7 + 601, j)) >= chance {
        return vec4<f32>(0.0);
    }
    let r = 0.008 + 0.017 * unit(hash2(i + 613, j * 3));
    let ou = r + (c - 2.0 * r) * unit(hash2(i * 5 + 617, j + 11));
    let ov = r + (c - 2.0 * r) * unit(hash2(i + 619, j * 5 + 13));
    let d = vec2<f32>(u - (f32(i) * c + ou), v - (f32(j) * c + ov)) / r;
    let q = length(d);
    let aa = max(fp, 1e-4) / r;
    let cover = (1.0 - smoothstep(1.0 - aa, 1.0 + aa, q)) * (1.0 - smoothstep(0.015, 0.05, fp));
    if cover <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(cover, unit(hash2(i + 631, j + 7)), d * 0.8 * min(q, 1.0));
}

// A pothole as the pixel sees it: cover (inside the outline, filtered),
// wall (1: the far wall is seen, 0: the bottom), the tilt of the normal in
// the strip frame (across, along), the broken rim around it and q, the
// distance from the middle (1 at the outline).
struct Hole {
    cover: f32,
    wall: f32,
    tilt: vec2<f32>,
    rim: f32,
    q: f32,
}

// The potholes of the pixel's cell on both sides
// (ocw_world::road_wear::pothole_in_cell). `vt`: the view direction in
// the strip frame (across, along, up), for the parallax of the bottom.
fn pothole(u: f32, du: f32, v: f32, dv: f32, half: f32, chance: f32, zone: f32, track: bool, vt: vec3<f32>) -> Hole {
    var out: Hole;
    out.cover = 0.0;
    out.wall = 0.0;
    out.tilt = vec2<f32>(0.0);
    out.rim = 0.0;
    out.q = 2.0;
    let k = i32(floor(v / 7.0));
    for (var side = 0; side < 2; side++) {
        if unit(hash2(k, 211 + side)) >= chance {
            continue;
        }
        let a = unit(hash2(k, 223 + side));
        var r = 0.12 + 0.33 * a * a;
        let vc = (f32(k) + 0.2 + 0.6 * unit(hash2(k, 227 + side))) * 7.0;
        if abs(v - vc) > 0.6 {
            continue;
        }
        let sgn = select(-1.0, 1.0, side == 1);
        let e = unit(hash2(k, 233 + side));
        var uc = 0.0;
        if track {
            uc = sgn * (0.8 + 0.2 * (e - 0.5));
            r = min(r, 0.35);
        } else if unit(hash2(k, 229 + side)) < 0.35 {
            uc = sgn * (half - 0.25 * r);
        } else {
            let span = max(min(zone, half - r) - r - 0.1, 0.0);
            uc = sgn * (half - r - 0.1 - span * e * e);
        }
        let d = vec2<f32>(u - uc, v - vc);
        // A ragged outline (the noise is seeded by the cell, not the tile).
        let seed = vec2<f32>(unit(hash2(k, 239 + side)) * 50.0, 0.0);
        let rr = r * (0.8 + 0.4 * vnoise(d * (2.0 / r) + seed));
        let q = length(d) / rr;
        if q > 1.3 {
            continue;
        }
        let aa = max(max(du, dv), 1e-3) / rr;
        let cover = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, q);
        // Parallax: where the view ray meets the bottom (3-5 cm deep).
        let depth = 0.03 + 0.05 * r;
        var off = vt.xy / max(vt.z, 0.12) * depth;
        let ol = length(off);
        if ol > 1.2 * r {
            off = off * (1.2 * r / ol);
        }
        let qb = length(d - off) / (0.78 * rr);
        let wall = smoothstep(0.85, 1.05, qb);
        let rim = (smoothstep(0.92, 1.0, q) - smoothstep(1.06, 1.25, q))
            * (1.0 - smoothstep(0.08, 0.2, max(du, dv)));
        if cover > out.cover {
            out.cover = cover;
            out.wall = wall;
            out.tilt = -normalize(d + vec2<f32>(1e-5, 0.0)) * 0.75 * wall;
            out.q = q;
        }
        out.rim = max(out.rim, rim);
    }
    return out;
}

// Tyre tracks: two per lane, 0.8 m either side of its middle.
fn tracks(u: f32, half: f32, lf: u32, lb: u32) -> f32 {
    var n = lf + lb;
    if n == 0u {
        n = select(1u, 2u, half >= 2.5);
    }
    let lane = 2.0 * half / f32(n);
    var t = 0.0;
    for (var i = 0u; i < n; i++) {
        let c = -half + (f32(i) + 0.5) * lane;
        let d1 = (u - (c - 0.8)) / 0.3;
        let d2 = (u - (c + 0.8)) / 0.3;
        t = max(t, max(exp(-d1 * d1), exp(-d2 * d2)));
    }
    return t;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );

    let wp = in.world_position.xyz;
    let dist = distance(wp, view.world_position.xyz);
    // Tangent frame and pixel footprint from screen derivatives (in
    // uniform control flow).
    let dp1 = dpdx(wp);
    let dp2 = dpdy(wp);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let du = max(length(vec2<f32>(duv1.x, duv2.x)), 1e-4);
    let dv = max(length(vec2<f32>(duv1.y, duv2.y)), 1e-4);

    let code = u32(round(in.uv_b.x));
    let layer = code & 7u;
    let kind = (code >> 3u) & 7u;
    let extra = code >> 6u;
    let lines = u32(round(in.uv_b.y));
    let half = in.color.a;
    let u = in.uv.x;
    let v = in.uv.y;

    var albedo = pbr_input.material.base_color.rgb;
    var rough = pbr_input.material.perceptual_roughness;
    var N = pbr_input.N;
    let geo_n = normalize(in.world_normal);

    // The kerb stones' top on the first centimetres of a sidewalk.
    var lay = layer;
    if kind == 4u && abs(u) - half < look.crossing_colour.w {
        lay = 6u;
    }
    // Cotangent frame: T along +u (across), B along +v (along the road).
    let dp2perp = cross(dp2, geo_n);
    let dp1perp = cross(geo_n, dp1);
    var T = dp2perp * duv1.x + dp1perp * duv2.x;
    var B = dp2perp * duv1.y + dp1perp * duv2.y;
    let inv = inverseSqrt(max(max(dot(T, T), dot(B, B)), 1e-12));
    T = T * inv;
    B = B * inv;
    if look.mode.x > 0.5 && kind != 5u && dist < look.mode.z {
        let s = look.sizes[lay / 4u][lay % 4u];
        let uv = in.uv / s;
        let a = textureSampleGrad(albedo_array, array_sampler, uv, i32(lay), duv1 / s, duv2 / s);
        let nt = textureSampleGrad(normal_array, array_sampler, uv, i32(lay), duv1 / s, duv2 / s).xy * 2.0 - 1.0;
        // GL normal maps: green up = image up = -v.
        let nw = normalize(geo_n + look.mode.w * (T * nt.x - B * nt.y));
        let t = smoothstep(look.mode.y, look.mode.z, dist);
        albedo = mix(a.rgb, albedo, t);
        rough = mix(clamp(a.a, 0.3, 1.0), rough, t);
        N = normalize(mix(nw, N, t));
    }

    // Wear on asphalt (ocw_world::road_wear, by the strip's grade): tyre
    // tracks, repaired patches, cracks and their sealing, alligator areas,
    // a little variation in tone (deterministic per position). The cracks
    // are applied after the markings (the paint cracks with the road).
    // The `road.damage` override (detail.z: 1 … 7 every road that grade,
    // 8 none); parts without a strip frame (grade 0) stay without.
    var grade = (extra >> 5u) & 7u;
    let forced = u32(round(look.detail.z));
    if forced > 0u && grade > 0u {
        grade = select(forced, 0u, forced >= 8u);
    }
    let track = ((extra >> 4u) & 1u) == 1u;
    let dmg = look.damage[grade];
    let rep = look.repair[grade];
    let loose = look.loose[grade];
    let concrete = ((extra >> 8u) & 1u) == 1u;
    let main_road = ((extra >> 9u) & 1u) == 1u;
    let damaged = look.line_colour.w > 0.0 && kind == 0u && grade > 0u
        && abs(u) <= half + 0.01 && dist < look.detail.y;
    var crack = 0.0;
    var seal = 0.0;
    var pc = 0.0;
    // The pixel's footprint: fine features it would fade away are skipped.
    let fpx = max(du, dv);
    // The wheel tracks (wear, loose stones, puddles), once.
    var tr = 0.0;
    if kind == 0u {
        tr = tracks(u, half, lines & 7u, (lines >> 3u) & 7u);
    }
    if kind == 0u && layer <= 1u && look.line_colour.w > 0.0 {
        let ts = select(0.1, rep.w, grade > 0u);
        albedo = albedo * (1.0 - ts * tr);
        rough = rough - 0.08 * tr;
        pc = patch_cov(half, u, du, v, dv, rep.x);
        albedo = mix(albedo, albedo * 0.8, pc);
        if damaged {
            var tc = vec2<f32>(0.0);
            var lc = vec2<f32>(0.0);
            if fpx < 0.3 {
                tc = transverse(u, du, v, dv, half, dmg.x, rep.y);
                lc = longitudinal(u, du, v, dv, half, dmg.y, rep.y);
            }
            if concrete {
                // Slabs: a sealed joint every 5 m and along the middle,
                // fewer random cracks (broken slabs).
                let jv = v - round(v / 5.0) * 5.0;
                let joints = max(band_cov(jv, max(dv, 1e-3), -0.007, 0.007), band_cov(u, max(du, 1e-3), -0.007, 0.007));
                tc = vec2<f32>(tc.x * 0.5, max(tc.y, joints));
            }
            let al = alligator(u, du, v, dv, half, dmg.z, track, look.detail.x > 0.5 && fpx < 0.12);
            // A patch is newer than the cracks around it.
            crack = max(max(tc.x, lc.x), al.y) * (1.0 - pc);
            seal = max(tc.y, lc.y) * (1.0 - pc);
            albedo = albedo * (1.0 - 0.12 * al.x * (1.0 - pc));
            rough = rough + 0.05 * al.x;
        }
        albedo = albedo * (0.94 + 0.12 * vnoise(wp.xz / 9.0));
        if concrete {
            // Concrete is a lighter, cooler grey than asphalt.
            let l = dot(albedo, vec3<f32>(0.3, 0.59, 0.11));
            albedo = mix(albedo, vec3<f32>(1.7 * l + 0.05), 0.75);
        }
    }
    // Unpaved roads (gravel, dirt): two ruts, darker and compacted, the
    // loose material lighter between them.
    if damaged && (layer == 2u || layer == 4u) {
        let d = (abs(u) - 0.8) / 0.28;
        let g = exp(-d * d);
        albedo = albedo * (1.0 - 0.18 * loose.y * g) * (1.0 + 0.08 * loose.y * (1.0 - g));
        let tilt = loose.y * 0.6 * (2.0 * d * g) * sign(u);
        N = normalize(N + T * tilt);
    }
    // Tracks: grass between the wheel ruts.
    if kind == 0u && ((extra >> 4u) & 1u) == 1u {
        let wobble = 0.12 * (vnoise(vec2<f32>(v * 0.4, 3.0)) - 0.5);
        let g = 1.0 - smoothstep(0.22, 0.42, abs(u + wobble));
        let grass = vec3<f32>(0.10, 0.16, 0.05) * (0.7 + 0.6 * vnoise(wp.xz * 2.3));
        albedo = mix(albedo, grass, g * 0.85);
        let r = (abs(u) - 0.8) / 0.25;
        albedo = albedo * (1.0 - 0.15 * exp(-r * r));
    }
    if kind == 3u || kind == 4u {
        // Kerb stones and pavers: the photographed textures are warm; the
        // concrete of the regions is a cooler grey.
        let l = dot(albedo, vec3<f32>(0.3, 0.59, 0.11));
        albedo = mix(vec3<f32>(l), albedo, 0.45);
    }
    if kind == 2u {
        // The verge: half-desaturated and darker, so it sits in any ground.
        let l = dot(albedo, vec3<f32>(0.3, 0.59, 0.11));
        albedo = mix(vec3<f32>(l), albedo, 0.5) * (0.72 + 0.16 * vnoise(wp.xz * 1.7));
    } else if kind == 1u {
        albedo = albedo * 0.8;
    }
    // Markings on the carriageway: on worn roads the paint is worn in
    // flecks and missing on the repaired patches.
    if kind == 0u {
        var m = markings(u, du, v, dv, half, lines, extra) * 0.92;
        if grade > 0u && max(m.x, m.y) > 0.0 {
            let tw = f32(grade - 1u) / 6.0;
            let fleck = smoothstep(0.55, 0.85, vnoise(vec2<f32>(u * 6.0, v * 1.5)));
            m = m * (1.0 - (0.1 + 0.45 * tw) * fleck) * (1.0 - smoothstep(0.4, 0.9, tw) * 0.7 * pc);
        }
        albedo = mix(albedo, look.line_colour.rgb, m.x);
        albedo = mix(albedo, look.cycle_colour.rgb, m.y);
        let cov = max(m.x, m.y);
        rough = mix(rough, 0.55, cov);
        N = normalize(mix(N, geo_n, cov));
    }
    // Cracks (dark, a little rough) and sealing lines (bitumen: dark and
    // glossy), over the paint too.
    albedo = mix(albedo, albedo * 0.32, crack);
    rough = rough + 0.1 * crack;
    let bitumen = vec3<f32>(0.035, 0.034, 0.033);
    albedo = mix(albedo, min(albedo * 0.6, bitumen + 0.4 * albedo), seal * 0.85);
    rough = mix(rough, 0.32, seal);
    // Potholes and edge breaks (asphalt, rarely on paving, hollows on
    // gravel and dirt; ocw_world::road_wear::surface_factors): a recess
    // drawn by parallax, normals and shading - the ridden surface is not
    // lowered.
    var hole: Hole;
    hole.cover = 0.0;
    hole.wall = 0.0;
    hole.tilt = vec2<f32>(0.0);
    hole.rim = 0.0;
    hole.q = 2.0;
    if damaged && layer <= 4u {
        var factor = 1.0;
        if layer == 3u {
            factor = 0.3;
        } else if layer == 2u || layer == 4u {
            factor = 1.5;
        }
        let V = normalize(view.world_position.xyz - wp);
        let vt = vec3<f32>(dot(V, T), dot(V, B), dot(V, geo_n));
        var zone = rep.z;
        if main_road {
            zone = min(zone, 1.2);
        }
        hole = pothole(u, du, v, dv, half, dmg.w * factor, zone, track, vt);
        if hole.cover > 0.0 || hole.rim > 0.0 {
            let grit = 0.75 + 0.5 * vnoise(vec2<f32>(u, v) * 40.0);
            let paved = layer <= 1u || layer == 3u;
            // The bottom: coarse, dusty aggregate in the wall's shadow; the
            // far wall: the broken asphalt edge, lit.
            let bottom = albedo * select(0.6, 0.45, paved) * grit;
            let wall = albedo * select(0.75, 0.62, paved) * (0.85 + 0.3 * grit);
            let inside = mix(bottom, wall, hole.wall);
            albedo = albedo * (1.0 + select(0.05, 0.16, paved) * hole.rim * (1.0 - hole.cover));
            albedo = mix(albedo, inside, hole.cover);
            rough = mix(rough, 0.95, hole.cover);
            let nh = normalize(geo_n + T * hole.tilt.x + B * hole.tilt.y);
            N = normalize(mix(N, nh, hole.cover));
        }
        // Loose stones and grit (bad roads; every unpaved road a few):
        // mostly at the edges and between the wheel tracks, swept out of
        // the tracks themselves.
        let unpaved = layer == 2u || layer == 4u;
        var chance = loose.x + select(0.0, 0.06, unpaved);
        if chance > 0.0 && fpx < 0.05 {
            chance = chance * (1.3 - 0.9 * tr) * (0.6 + 0.8 * smoothstep(0.5 * half, half, abs(u)));
            let st = stone(u, v, max(du, dv), chance);
            if st.x > 0.0 {
                var c = mix(vec3<f32>(0.10, 0.098, 0.095), vec3<f32>(0.42, 0.40, 0.37), st.y);
                if unpaved {
                    c = albedo * mix(0.6, 1.6, st.y);
                }
                albedo = mix(albedo, c, st.x);
                rough = mix(rough, 0.75, st.x);
                let ns = normalize(geo_n + T * st.z + B * st.w);
                N = normalize(mix(N, ns, st.x));
            }
        }
    }
    var alpha = pbr_input.material.base_color.a;
    if kind == 5u {
        let c = crossing(u, du, v, dv, half, lines & 3u, (lines >> 2u) & 3u);
        albedo = select(look.line_colour.rgb, look.crossing_colour.rgb, c.y > c.x);
        alpha = max(c.x, c.y) * 0.92;
        rough = 0.55;
        N = geo_n;
    }
    // The weather (spec §14 Phase 9, S4; `ocw_weather::ground`): a wet
    // road is darker and glossy; when the puddles fill, water stands in
    // the hollows (the low spots of a noise relief, the worn wheel ruts,
    // the edges) as dark mirrors. Snow lies on the verges only (roads are
    // cleared, spec 9C).
    let wet = look.weather.x;
    if wet > 0.0 && kind != 1u && kind != 5u {
        albedo = albedo * mix(1.0, select(0.55, 0.72, kind == 2u), wet);
        rough = mix(rough, select(0.2, 0.42, kind == 2u), wet);
        N = normalize(mix(N, geo_n, wet * 0.6));
        let pud = look.weather.y;
        if pud > 0.0 && (kind == 0u || kind == 4u) {
            var hollow = vnoise(wp.xz / 3.1) * 0.65 + vnoise(wp.xz / 0.9 + vec2<f32>(7.3, 1.9)) * 0.35;
            if kind == 0u {
                hollow = hollow + 0.18 * tr
                    + 0.12 * smoothstep(half * 0.7, half, abs(u));
            }
            let level = 1.0 - 0.5 * pud;
            let water = smoothstep(level, level + 0.04, hollow) * smoothstep(0.0, 0.2, pud);
            albedo = mix(albedo, albedo * 0.45, water);
            rough = mix(rough, 0.04, water);
            N = normalize(mix(N, geo_n, water));
        }
        // Water stands in the potholes first: from a well wet road on, and
        // to the rim when the puddles fill.
        let fill = clamp(max(1.6 * pud, 1.4 * (wet - 0.3)), 0.0, 1.0);
        if hole.cover > 0.0 && fill > 0.0 {
            let water = hole.cover * (1.0 - smoothstep(0.55 + 0.4 * fill, 0.65 + 0.4 * fill, hole.q))
                * smoothstep(0.0, 0.25, fill);
            albedo = mix(albedo, albedo * 0.4, water);
            rough = mix(rough, 0.03, water);
            N = normalize(mix(N, geo_n, water));
        }
    }
    let snow = max(look.weather.z, clamp(look.season.x + look.season.y * wp.y, 0.0, 1.0));
    if snow > 0.0 && kind == 2u {
        let s = snow * (0.55 + 0.45 * vnoise(wp.xz * 0.8));
        albedo = mix(albedo, vec3<f32>(0.86, 0.89, 0.93), s);
        rough = mix(rough, 0.6, s);
    }
    pbr_input.material.base_color = vec4<f32>(albedo, alpha);
    pbr_input.material.perceptual_roughness = clamp(rough, 0.089, 1.0);
    pbr_input.N = N;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
