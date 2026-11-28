// Far terrain (spec 4.5 *Far terrain*, v6.58): the vertex colours of the
// 100 m grid (the land use's colours, the canopy, the satellite colour)
// with a texture where they would be flat: the forest's crowns and the
// stands they grow in, the stones and outcrops of the alpine ground. The
// detail fades out where a pixel covers more than its size (no shimmer),
// so far away only the stands' larger patches are left.
//
// Vertex data (`far_terrain::far_tile_arrays`): uv.x = the trees' cover
// (0 – 1), uv.y = the alpine share (0 – 1).

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::view,
};
import ocw_render::terrain_shadow;

struct FarLook {
    // x: crown contrast, y: stand contrast, z: stone contrast, w: the
    // backlit canopy's strength.
    params: vec4<f32>,
    // xyz: the direction to the sun (the key light); w: the share of the
    // backlit canopy on open ground.
    sun: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> far: FarLook;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Noise of feature size `size` (m), faded to its mean 0.5 where a pixel
// (`footprint`, m) is larger than a third of it.
fn detail(p: vec2<f32>, size: f32, footprint: f32) -> f32 {
    let keep = 1.0 - smoothstep(size * 0.15, size * 0.5, footprint);
    return mix(0.5, vnoise(p / size), keep);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let wp = in.world_position.xyz;
    let footprint = length(fwidth(wp.xz));
    let trees = clamp(in.uv.x, 0.0, 1.0);
    let alpine = clamp(in.uv.y, 0.0, 1.0);
    var c = pbr_input.material.base_color.rgb;

    // Crowns (8 m) with the shade between them, in stands of 60 – 150 m.
    let crowns = detail(wp.xz + vec2<f32>(17.0, 3.0), 8.0, footprint);
    let stands = detail(wp.xz, 90.0, footprint) * 0.6 + detail(wp.xz + vec2<f32>(400.0, 0.0), 230.0, footprint) * 0.4;
    let canopy = 1.0 + trees * (far.params.x * (crowns - 0.5) * 2.0 + far.params.y * (stands - 0.5) * 2.0);
    // The alpine ground: pale stones and dark outcrops at 15 and 70 m.
    let stones = detail(wp.xz + vec2<f32>(-91.0, 37.0), 15.0, footprint) * 0.5
        + detail(wp.xz + vec2<f32>(5.0, -230.0), 70.0, footprint) * 0.5;
    let rock = 1.0 + alpine * (1.0 - trees) * far.params.z * (stones - 0.5) * 2.0;
    c = c * canopy * rock;
    // Looking into a low sun a forest shows the shaded sides of its crowns
    // (`far_terrain::backlit_canopy`, v6.72).
    let low = 1.0 - smoothstep(0.14, 0.42, far.sun.y);
    let v = wp.xz - view.world_position.xz;
    let s = far.sun.xz;
    if low > 0.0 && dot(v, v) > 1e-6 && dot(s, s) > 1e-8 {
        let towards = max(dot(normalize(v), normalize(s)), 0.0);
        let cover = trees + (1.0 - trees) * far.sun.w;
        c = c * (1.0 - cover * far.params.w * towards * towards * low);
    }
    pbr_input.material.base_color = vec4<f32>(c, pbr_input.material.base_color.a);

    var out: FragmentOutput;
    out.color = terrain_shadow::lit(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
