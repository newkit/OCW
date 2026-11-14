// Precipitation (spec §14 Phase 9, S4): every quad is a particle whose
// place comes from its seed, the accumulated fall and drift, wrapped into a
// box around the camera; drops are streaks along their motion relative to
// the camera, flakes and hail are round and face the camera. Quads beyond
// the count are collapsed.

import bevy_pbr::render::{
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
};

struct Precip {
    // xyz: accumulated motion (m), w: particles drawn.
    offset: vec4<f32>,
    // x: half box xz, y: half box y, z: width, w: streak length.
    shape: vec4<f32>,
    // xyz: velocity relative to the camera, w: opacity.
    motion: vec4<f32>,
    // rgb: colour (before exposure), w: kind.
    colour: vec4<f32>,
    // x: time, y: sway (m), z: visibility (m), w: quads in the mesh.
    misc: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> precip: Precip;

struct Vertex {
    @location(0) seed: vec3<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) extra: vec2<f32>,
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) fade: f32,
};

@vertex
fn vertex(v: Vertex) -> Out {
    var out: Out;
    out.corner = v.corner;
    out.fade = 0.0;
    if v.extra.x * precip.misc.w >= precip.offset.w {
        // Beyond the count: outside the clip volume, no area.
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let h = vec3<f32>(precip.shape.x, precip.shape.y, precip.shape.x);
    let size = 2.0 * h;
    let eye = view.world_position.xyz;
    // Fixed in the world, moving with the offset, wrapped around the eye.
    let local = fract((v.seed * size + precip.offset.xyz - eye) / size) * size - h;
    var c = eye + local;
    let kind = u32(precip.colour.w + 0.5);
    let t = precip.misc.x;
    if precip.misc.y > 0.0 {
        let ph = v.extra.y * 40.0;
        c = c + precip.misc.y * vec3<f32>(sin(t * 1.3 + ph), 0.0, cos(t * 1.1 + ph * 1.7));
    }
    let to_eye = eye - c;
    let dist = max(length(to_eye), 1e-3);
    let width = precip.shape.z * (0.75 + 0.5 * v.extra.y);
    var axis: vec3<f32>;
    var side: vec3<f32>;
    if kind == 2u || kind == 4u || precip.shape.w <= precip.shape.z * 1.01 {
        // Round: facing the camera.
        side = view.world_from_view[0].xyz * width;
        axis = view.world_from_view[1].xyz * width;
    } else {
        let m = precip.motion.xyz;
        let dir = m / max(length(m), 1e-3);
        axis = dir * precip.shape.w * (0.8 + 0.4 * v.extra.y);
        var s = cross(dir, to_eye / dist);
        if length(s) < 1e-3 {
            s = view.world_from_view[0].xyz;
        }
        side = normalize(s) * width;
    }
    let p = c + axis * (v.corner.y - 0.5) + side * (v.corner.x - 0.5);
    out.clip = position_world_to_clip(p);
    // Soft at the lens, out towards the box's edge and in fog
    // (`ocw_weather::particles::Particles::fade`).
    let near = clamp((dist - 0.4) / 1.2, 0.0, 1.0);
    let edge = clamp(1.0 - (dist / h.x - 0.7) / 0.3, 0.0, 1.0);
    let fog = exp(-3.912 * dist / max(precip.misc.z, 1.0));
    out.fade = near * edge * fog;
    return out;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let kind = u32(precip.colour.w + 0.5);
    let q = in.corner * 2.0 - 1.0;
    var a: f32;
    if kind == 2u || kind == 4u {
        a = 1.0 - smoothstep(0.45, 1.0, length(q));
    } else {
        let across = 1.0 - abs(q.x);
        let along = 1.0 - smoothstep(0.6, 1.0, abs(q.y));
        a = across * across * along;
    }
    let alpha = a * precip.motion.w * in.fade;
    if alpha < 0.003 {
        discard;
    }
    return vec4<f32>(precip.colour.rgb * view.exposure, alpha);
}
