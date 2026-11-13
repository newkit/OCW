// Road surfaces (spec §4.6 step 6, Phase 8 T2): the standard PBR fragment
// shader with the road material of the strip's surface (layer in uv_b.x,
// uv = across / along in metres). Edge faces (uv_b.y = 1) keep their flat
// colour. Independent of the light setup.

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
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> look: RoadLook;
@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var albedo_array: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102)
var array_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103)
var normal_array: texture_2d_array<f32>;

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
    // Tangent frame from screen derivatives (in uniform control flow).
    let dp1 = dpdx(wp);
    let dp2 = dpdy(wp);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    if look.mode.x > 0.5 && in.uv_b.y < 0.5 && dist < look.mode.z {
        let layer = u32(in.uv_b.x + 0.5);
        let s = look.sizes[layer / 4u][layer % 4u];
        let uv = in.uv / s;
        let a = textureSampleGrad(albedo_array, array_sampler, uv, i32(layer), duv1 / s, duv2 / s);
        let nt = textureSampleGrad(normal_array, array_sampler, uv, i32(layer), duv1 / s, duv2 / s).xy * 2.0 - 1.0;
        let N = normalize(in.world_normal);
        // Cotangent frame: T along +u (across), B along +v (along the road).
        let dp2perp = cross(dp2, N);
        let dp1perp = cross(N, dp1);
        var T = dp2perp * duv1.x + dp1perp * duv2.x;
        var B = dp2perp * duv1.y + dp1perp * duv2.y;
        let inv = inverseSqrt(max(max(dot(T, T), dot(B, B)), 1e-12));
        T = T * inv;
        B = B * inv;
        // GL normal maps: green up = image up = -v.
        let nw = normalize(N + look.mode.w * (T * nt.x - B * nt.y));
        let t = smoothstep(look.mode.y, look.mode.z, dist);
        pbr_input.material.base_color = vec4<f32>(mix(a.rgb, pbr_input.material.base_color.rgb, t), 1.0);
        pbr_input.material.perceptual_roughness = mix(clamp(a.a, 0.3, 1.0), pbr_input.material.perceptual_roughness, t);
        pbr_input.N = normalize(mix(nw, pbr_input.N, t));
    }

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
