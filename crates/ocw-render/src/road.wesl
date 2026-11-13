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
    // patch chance asphalt, worn; crack chance worn, asphalt.
    wear: vec4<f32>,
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

// A transverse crack, gone when the pixel is wider than a few centimetres.
fn crack_cov(u: f32, du: f32, v: f32, dv: f32, half: f32, chance: f32) -> f32 {
    let k = i32(floor(v / 11.0));
    let h = hash2(k, 101);
    if unit(h) >= chance {
        return 0.0;
    }
    let v0 = (f32(k) + 0.15 + 0.7 * unit(hash2(k, 103))) * 11.0
        + 0.15 * sin(u * 1.7 + unit(h) * 40.0) + 0.04 * sin(u * 6.1);
    let reach = half * (0.4 + 0.6 * unit(hash2(k, 107)));
    let fade = 1.0 - smoothstep(0.015, 0.05, dv);
    return band_cov(v, max(dv, 1e-3), v0 - 0.008, v0 + 0.008) * band_cov(u, du, -reach, reach) * fade;
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
    if look.mode.x > 0.5 && kind != 5u && dist < look.mode.z {
        let s = look.sizes[lay / 4u][lay % 4u];
        let uv = in.uv / s;
        let a = textureSampleGrad(albedo_array, array_sampler, uv, i32(lay), duv1 / s, duv2 / s);
        let nt = textureSampleGrad(normal_array, array_sampler, uv, i32(lay), duv1 / s, duv2 / s).xy * 2.0 - 1.0;
        // Cotangent frame: T along +u (across), B along +v (along the road).
        let dp2perp = cross(dp2, geo_n);
        let dp1perp = cross(geo_n, dp1);
        var T = dp2perp * duv1.x + dp1perp * duv2.x;
        var B = dp2perp * duv1.y + dp1perp * duv2.y;
        let inv = inverseSqrt(max(max(dot(T, T), dot(B, B)), 1e-12));
        T = T * inv;
        B = B * inv;
        // GL normal maps: green up = image up = -v.
        let nw = normalize(geo_n + look.mode.w * (T * nt.x - B * nt.y));
        let t = smoothstep(look.mode.y, look.mode.z, dist);
        albedo = mix(a.rgb, albedo, t);
        rough = mix(clamp(a.a, 0.3, 1.0), rough, t);
        N = normalize(mix(nw, N, t));
    }

    // Wear on asphalt: tyre tracks, repaired patches, cracks, a little
    // variation in tone (deterministic per position).
    if kind == 0u && layer <= 1u && look.line_colour.w > 0.0 {
        let tr = tracks(u, half, lines & 7u, (lines >> 3u) & 7u);
        albedo = albedo * (1.0 - 0.1 * tr);
        rough = rough - 0.08 * tr;
        let worn = layer == 1u;
        let pc = patch_cov(half, u, du, v, dv, select(look.wear.x, look.wear.y, worn));
        albedo = mix(albedo, albedo * 0.8, pc);
        let cc = crack_cov(u, du, v, dv, half, select(look.wear.w, look.wear.z, worn));
        albedo = mix(albedo, albedo * 0.35, cc);
        albedo = albedo * (0.94 + 0.12 * vnoise(wp.xz / 9.0));
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
    // Markings on the carriageway.
    if kind == 0u {
        let m = markings(u, du, v, dv, half, lines, extra) * 0.92;
        albedo = mix(albedo, look.line_colour.rgb, m.x);
        albedo = mix(albedo, look.cycle_colour.rgb, m.y);
        let cov = max(m.x, m.y);
        rough = mix(rough, 0.55, cov);
        N = normalize(mix(N, geo_n, cov));
    }
    var alpha = pbr_input.material.base_color.a;
    if kind == 5u {
        let c = crossing(u, du, v, dv, half, lines & 3u, (lines >> 2u) & 3u);
        albedo = select(look.line_colour.rgb, look.crossing_colour.rgb, c.y > c.x);
        alpha = max(c.x, c.y) * 0.92;
        rough = 0.55;
        N = geo_n;
    }
    pbr_input.material.base_color = vec4<f32>(albedo, alpha);
    pbr_input.material.perceptual_roughness = clamp(rough, 0.089, 1.0);
    pbr_input.N = N;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
