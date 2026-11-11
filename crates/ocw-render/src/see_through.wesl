// See-through objects (trees): the standard PBR fragment shader, but
// fragments right in front of the lens or between the camera and the rider
// are discarded in an ordered 4 × 4 dither (screen-door transparency).
// `see_through_keep` mirrors `see_through::keep` in Rust.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
};

struct SeeThroughFade {
    // xyz: the rider's head; w: gap before it where the sight fade ends
    // (negative = no focus).
    focus: vec4<f32>,
    // x: near fade radius, y/z: sight radius at the eye/at the focus,
    // w: ramp width.
    shape: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> see_through: SeeThroughFade;

fn see_through_keep(p: vec3<f32>, eye: vec3<f32>, fade: SeeThroughFade) -> f32 {
    let shape = fade.shape;
    let ramp = max(shape.w, 1e-3);
    var keep = clamp((distance(p, eye) - shape.x) / ramp, 0.0, 1.0);
    let focus = fade.focus;
    if focus.w >= 0.0 {
        let to_focus = focus.xyz - eye;
        let len = length(to_focus);
        if len > 1e-3 {
            let dir = to_focus / len;
            let along = dot(p - eye, dir);
            if along > 0.0 {
                let off_axis = length(p - eye - dir * along);
                let radius = shape.y + (shape.z - shape.y) * min(along / len, 1.0);
                let sight = clamp((off_axis - radius) / ramp, 0.0, 1.0);
                let end = clamp((len - focus.w - along) / ramp, 0.0, 1.0);
                keep = min(keep, 1.0 - end * (1.0 - sight));
            }
        }
    }
    return keep;
}

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

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let keep = see_through_keep(in.world_position.xyz, view.world_position, see_through);
    if keep < bayer4(in.position.xy) {
        discard;
    }

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
