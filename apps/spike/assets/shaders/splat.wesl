// Splat terrain material (spec §4.6): four texture layers blended per pixel
// by weights from the vertex colour (r = grass, g = forest floor, b = rock,
// a = verge), then Bevy's standard PBR lighting, shadows and fog.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var splat_textures: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var splat_sampler: sampler;

// Texture repeat in metres. Floating-origin shifts are whole kilometres, a
// multiple of this, so the pattern does not jump when the origin moves.
const TEXTURE_METRES: f32 = 4.0;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let uv = in.world_position.xz / TEXTURE_METRES;
    let w = max(in.color, vec4<f32>(0.0));
    var c = textureSample(splat_textures, splat_sampler, uv, 0).rgb * w.r
        + textureSample(splat_textures, splat_sampler, uv, 1).rgb * w.g
        + textureSample(splat_textures, splat_sampler, uv, 2).rgb * w.b
        + textureSample(splat_textures, splat_sampler, uv, 3).rgb * w.a;
    let total = max(w.r + w.g + w.b + w.a, 0.0001);
    pbr_input.material.base_color = vec4<f32>(c / total, 1.0);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
