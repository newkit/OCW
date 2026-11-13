// Houses (spec §14 Phase 8 T4): the standard PBR fragment shader with the
// building materials of the library (plaster, timber, stone, brick,
// concrete, metal, clay tiles, slates; glass untextured) and procedural
// façades: windows per bay and storey, one door, shutters, shop fronts, lit
// windows at night. Vertex data (ocw-render `houses`): uv = (along, up) in
// metres; uv_b.x = lower layer + 16 × upper layer; uv_b.y = window style +
// 1 + 16 × (storey height in dm + 64 × storeys); colour = the lower layer's
// colour (linear), alpha = door bay + 1. Independent of the light setup.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, visibility_range_dither},
    mesh_view_bindings::view,
};

struct BuildingLook {
    // x: textured; y, z: texture fade start, end (m); w: normal strength.
    mode: vec4<f32>,
    // x, y: window fade start, end (m); z: night (lit windows, 0-1);
    // w: brightness of a lit window (emissive, physical units).
    facade: vec4<f32>,
    // Repeat size (m) of the 16 layers.
    sizes: array<vec4<f32>, 4>,
    // Mean albedo of each layer (linear): the upper floors' colour, and
    // what the texture is divided by to take the vertex colour.
    means: array<vec4<f32>, 16>,
    // Per window style: (bay, width, height, sill), (frame rgb, flags:
    // 1 shutters, 2 shop front), (shutter rgb, door width), (door rgb,
    // door height).
    windows: array<vec4<f32>, 64>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> look: BuildingLook;
@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var albedo_array: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102)
var array_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103)
var normal_array: texture_2d_array<f32>;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// Coverage of a box given its signed distance (negative inside), smoothed
// over `aa` (anti-aliasing).
fn cover(d: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(-aa, aa, d);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
@if(VISIBILITY_RANGE_DITHER)
    visibility_range_dither(in.position, in.visibility_range_dither);

    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let wp = in.world_position.xyz;
    let dist = distance(wp, view.world_position.xyz);
    // Derivatives in uniform control flow.
    let dp1 = dpdx(wp);
    let dp2 = dpdy(wp);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let fw = fwidth(in.uv);

    let lcode = u32(in.uv_b.x + 0.5);
    let fcode = u32(in.uv_b.y + 0.5);
    let lower = lcode % 16u;
    let upper = lcode / 16u;
    let win = fcode % 16u;
    let storey_h = f32((fcode / 16u) % 64u) / 10.0;
    let levels = fcode / 1024u;
    let door_bay = u32(in.color.a + 0.5);

    // The floors above the first storey (timber upper floors).
    var layer = lower;
    var tone = in.color.rgb;
    if upper != lower && storey_h > 0.0 && in.uv.y > storey_h {
        layer = upper;
        tone = look.means[upper].rgb;
    }

    var albedo = tone;
    var rough = pbr_input.material.perceptual_roughness;
    var n = normalize(in.world_normal);
    if layer == 15u {
        // Glass: untextured, glossy.
        rough = 0.12;
    } else if look.mode.x > 0.5 && dist < look.mode.z {
        let s = look.sizes[layer / 4u][layer % 4u];
        // Image up = +v (up the wall or slope): the texture's v runs down.
        let flip = vec2<f32>(1.0, -1.0);
        let uv = in.uv * flip / s;
        let g1 = duv1 * flip / s;
        let g2 = duv2 * flip / s;
        let a = textureSampleGrad(albedo_array, array_sampler, uv, i32(layer), g1, g2);
        let nt = textureSampleGrad(normal_array, array_sampler, uv, i32(layer), g1, g2).xy * 2.0 - 1.0;
        // The texture takes the face's colour: divided by its own mean.
        let mean = max(look.means[layer].rgb, vec3<f32>(0.02));
        let tinted = clamp(a.rgb * tone / mean, vec3<f32>(0.0), vec3<f32>(1.0));
        let t = smoothstep(look.mode.y, look.mode.z, dist);
        albedo = mix(tinted, tone, t);
        rough = mix(clamp(a.a, 0.35, 1.0), rough, t);
        // Cotangent frame: T along +u, B along +v (up the wall or slope).
        let dp2perp = cross(dp2, n);
        let dp1perp = cross(n, dp1);
        var tt = dp2perp * duv1.x + dp1perp * duv2.x;
        var bb = dp2perp * duv1.y + dp1perp * duv2.y;
        let inv = inverseSqrt(max(max(dot(tt, tt), dot(bb, bb)), 1e-12));
        tt = tt * inv;
        bb = bb * inv;
        // GL normal maps: green up = image up = +v here (up the wall).
        let nw = normalize(n + look.mode.w * (tt * nt.x + bb * nt.y));
        n = normalize(mix(nw, n, t));
    }

    // Procedural façade: windows per bay and storey, one door.
    var emissive = vec3<f32>(0.0);
    if win > 0u && levels > 0u && storey_h > 0.0 && dist < look.facade.y {
        let w = (win - 1u) * 4u;
        let p0 = look.windows[w];
        let p1 = look.windows[w + 1u];
        let p2 = look.windows[w + 2u];
        let p3 = look.windows[w + 3u];
        let flags = u32(p1.w + 0.5);
        let bay = max(p0.x, 0.5);
        let cell = floor(in.uv.x / bay);
        let lu = in.uv.x - (cell + 0.5) * bay;
        let storey = floor(in.uv.y / storey_h);
        let lv = in.uv.y - storey * storey_h;
        let fade = 1.0 - smoothstep(look.facade.x, look.facade.y, dist);
        let aa = max(max(fw.x, fw.y), 0.01);
        if storey >= 0.0 && storey < f32(levels) {
            var is_door = 0.0;
            if storey < 0.5 && door_bay > 0u && u32(cell + 0.5) + 1u == door_bay {
                let d = max(abs(lu) - p2.w * 0.5, lv - p3.w);
                is_door = cover(d, aa);
                albedo = mix(albedo, p3.rgb, is_door * fade);
                rough = mix(rough, 0.6, is_door * fade);
                n = normalize(mix(n, normalize(in.world_normal), is_door));
            }
            var ww = p0.y;
            var wh = p0.z;
            var sill = p0.w;
            if (flags & 2u) != 0u && storey < 0.5 {
                // Shop front on the ground floor.
                ww = bay * 0.8;
                sill = 0.45;
                wh = min(storey_h - 0.9, 2.6);
            }
            if ww > 0.0 && wh > 0.0 {
                let du = abs(lu) - ww * 0.5;
                let dv = max(sill - lv, lv - (sill + wh));
                let whole = cover(max(du, dv), aa) * (1.0 - is_door);
                let glass = cover(max(du, dv) + 0.07, aa) * (1.0 - is_door);
                if (flags & 1u) != 0u && !((flags & 2u) != 0u && storey < 0.5) {
                    // Shutters left and right of the window.
                    let shutter = cover(max(abs(abs(lu) - ww * 0.75) - ww * 0.25, dv), aa) * (1.0 - is_door);
                    albedo = mix(albedo, p2.rgb, shutter * fade);
                    n = normalize(mix(n, normalize(in.world_normal), shutter));
                }
                // Panes: dark, a little different behind each window
                // (curtains, rooms), some lit at night.
                let h = hash2(vec2<f32>(cell + in.color.r * 97.0, storey + in.color.g * 131.0));
                let pane = vec3<f32>(0.035, 0.045, 0.055) + vec3<f32>(0.05, 0.045, 0.04) * h;
                albedo = mix(albedo, p1.rgb, (whole - glass) * fade);
                albedo = mix(albedo, pane, glass * fade);
                rough = mix(rough, 0.08, glass * fade);
                n = normalize(mix(n, normalize(in.world_normal), whole));
                let lit = step(h, 0.45) * look.facade.z;
                emissive = vec3<f32>(1.0, 0.72, 0.42) * look.facade.w * lit * glass * fade;
            }
        }
    }

    pbr_input.material.base_color = vec4<f32>(albedo, 1.0);
    pbr_input.material.perceptual_roughness = rough;
    pbr_input.material.emissive = vec4<f32>(emissive, 1.0);
    pbr_input.N = n;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
