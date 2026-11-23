// The shadows of see-through objects (trees): the standard prepass, but a
// part with a vertex alpha below 1 – the see-through crown of twigs of a
// bare tree, a thinning autumn crown (`flora::crown_form`) – casts only that
// share of its shadow, in the same ordered 4 x 4 dither as the main pass
// (`see_through.wesl`). Used where the shadow pass needs a fragment shader:
// the tree material is alpha-masked (`see_through::with_crown_alpha`), and
// without depth clipping (WebGL 2) for every see-through material.

import bevy_pbr::prepass::io as prepass_io;

// Ordered dither threshold in (0, 1) for a pixel (4 × 4 Bayer matrix).
fn bayer4(frag_xy: vec2<f32>) -> f32 {
    var m = array<f32, 16>(
        0.0, 8.0, 2.0, 10.0,
        12.0, 4.0, 14.0, 6.0,
        3.0, 11.0, 1.0, 9.0,
        15.0, 7.0, 13.0, 5.0,
    );
    let c = vec2<u32>(floor(frag_xy)) % 4u;
    return (m[c.y * 4u + c.x] + 0.5) / 16.0;
}

// The share of a part's shadow that is cast (its vertex alpha). No
// cross-fade dither of the visibility ranges here: it would be measured
// from the light's view, not the camera's, and swap the near trees' shadows
// for the far models' (as the shadow pass without a fragment shader, both
// levels cast theirs).
fn crown_shadow(in: prepass_io::VertexOutput) {
@if(VERTEX_COLORS) {
    if in.color.a < bayer4(in.position.xy) {
        discard;
    }
}
}

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: prepass_io::VertexOutput) -> prepass_io::FragmentOutput {
    crown_shadow(in);
    var out: prepass_io::FragmentOutput;
@if(NORMAL_PREPASS)
    out.normal = vec4(in.world_normal * 0.5 + vec3(0.5), 1.0);
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
    out.frag_depth = in.unclipped_depth;
    return out;
}
@else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    crown_shadow(in);
}
