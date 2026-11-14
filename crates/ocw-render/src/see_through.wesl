// See-through objects (trees): the standard PBR fragment shader, but
// fragments right in front of the lens or between the camera and the rider
// are discarded in an ordered 4 × 4 dither (screen-door transparency).
// `see_through_keep` mirrors `see_through::keep` in Rust.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex},
    mesh_functions,
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
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
    // The weather: x / z wind direction, y sway amount (0-1), w snow.
    wind: vec4<f32>,
    // x: 0 = height above the foot in uv.x, 1 = local y + y.
    sway: vec4<f32>,
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

// The sway (spec §14 Phase 9, S4; `see_through::sway_offset`): a lean
// downwind and an oscillation growing with the height above the foot; the
// phase follows the place, so gusts run through a forest as a wave. The
// shadow pass keeps the trees still (cheap; the shadows are soft).
fn sway_offset(height: f32, tree: f32, amount: f32, phase: f32) -> f32 {
    if amount <= 0.0 || height <= 0.3 {
        return 0.0;
    }
    let k = min(height / max(tree, 1.0), 1.5);
    let bend = amount * k * k * (0.55 + 0.45 * sin(phase)) * 1.6;
    return bend + 0.06 * amount * k * sin(phase * 4.3);
}

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
@if(VERTEX_NORMALS)
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_in.instance_index);
    var wp = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    var height = vertex.position.y + see_through.sway.y;
@if(VERTEX_UVS_A) {
    if see_through.sway.x < 0.5 {
        height = vertex.uv.x;
    }
}
    let amount = see_through.wind.y;
    if amount > 0.0 {
        let phase = globals.time * (1.3 + 0.8 * amount) + dot(wp.xz, vec2<f32>(0.021, 0.034));
        let d = sway_offset(height, 9.0, amount, phase);
        wp = vec4<f32>(wp.x + see_through.wind.x * d, wp.y - 0.08 * d * d, wp.z + see_through.wind.z * d, wp.w);
    }
    out.world_position = wp;
    out.position = position_world_to_clip(wp.xyz);
@if(VERTEX_UVS_A)
    out.uv = vertex.uv;
@if(VERTEX_UVS_B)
    out.uv_b = vertex.uv_b;
@if(VERTEX_TANGENTS)
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex_in.instance_index);
@if(VERTEX_COLORS)
    out.color = vertex.color;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex_in.instance_index, world_from_local[3]);
    return out;
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
    // Snow on the crowns' upper sides (spec §14 Phase 9, S4).
    let snow = see_through.wind.w;
    if snow > 0.0 {
        let up = normalize(in.world_normal).y;
        let s = smoothstep(0.05, 0.45, up) * snow * 0.85;
        let c = mix(pbr_input.material.base_color.rgb, vec3<f32>(0.85, 0.88, 0.92), s);
        pbr_input.material.base_color = vec4<f32>(c, pbr_input.material.base_color.a);
    }
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
