// Spray behind the rear wheel on a wet road (spec §14 Phase 9, S4): puffs
// of mist thrown up and back, each on its own cycle, in the avatar's frame
// (the entity's scale is the amount); camera-facing, soft.

import bevy_pbr::render::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
};

struct Spray {
    // rgb: colour (before exposure), w: opacity.
    colour: vec4<f32>,
    // x: time (s).
    misc: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> spray: Spray;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) seed: vec3<f32>,
    @location(1) corner: vec2<f32>,
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) alpha: f32,
};

@vertex
fn vertex(v: Vertex) -> Out {
    var out: Out;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let a = fract(spray.misc.x * 1.7 + v.seed.x);
    let s = v.seed;
    // The avatar faces -z: behind the rear wheel is +z.
    let local = vec3<f32>(
        (s.y - 0.5) * (0.15 + 1.3 * a),
        0.08 + (0.5 + 0.7 * s.z) * a - 0.45 * a * a,
        0.5 + (0.8 + 1.4 * s.z) * a,
    );
    let c = (world_from_local * vec4<f32>(local, 1.0)).xyz;
    let scale = length(world_from_local[0].xyz);
    let size = (0.07 + 0.42 * a) * scale;
    let side = view.world_from_view[0].xyz * size;
    let up = view.world_from_view[1].xyz * size;
    let p = c + side * (v.corner.x - 0.5) + up * (v.corner.y - 0.5);
    out.clip = position_world_to_clip(p);
    out.corner = v.corner;
    out.alpha = (1.0 - a) * smoothstep(0.0, 0.12, a);
    return out;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let r = length(in.corner * 2.0 - 1.0);
    let soft = 1.0 - smoothstep(0.0, 1.0, r);
    let alpha = soft * soft * in.alpha * spray.colour.w;
    if alpha < 0.003 {
        discard;
    }
    return vec4<f32>(spray.colour.rgb * view.exposure, alpha);
}
