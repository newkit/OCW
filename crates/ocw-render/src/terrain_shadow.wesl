// Terrain shadows (spec 4.5 *Terrain shadows*, v7.15): the shadow of the
// mountains from the height field, for every surface that is lit by the
// sun – the near and far terrain, roads, trees, houses, water, riders'
// gear, vehicles. `terrain_shadow::TerrainShadowMap` keeps the shadow line
// of the whole world (`ocw_world::terrain_shadow`) in an RG16F texture in
// world x / z: r the height (m above `params.y`) above which a point is in
// the sun, g the penumbra's half height (m). A fragment below the line
// loses the sun's direct light; above it (a crown, a roof over a shaded
// slope) it keeps it.
//
// With the sun's shadow maps on, the visibility goes into Bevy's
// `directional_shadow_factor` (it multiplies the cascades' shadow, so the
// trees and riders near the camera still cast). Without them (the
// phones, `shadows off`) the sun's direct term is taken out after the
// lighting: the same light, the same answer.

import bevy_pbr::render::{
    pbr_types::PbrInput,
    pbr_functions::{apply_pbr_lighting, calculate_diffuse_color, calculate_F0_dielectric},
    pbr_lighting as lighting,
    pbr_lighting::{LAYER_BASE, LAYER_CLEARCOAT},
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
};

struct TerrainShadow {
    // uv = world xz * scale.xy + scale.zw (texel centres on the nodes).
    scale: vec4<f32>,
    // x: on (1) / off (0); y: the height the texture's lines count from
    // (m); z: strength (0 … 1); w: unused.
    params: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(120)
var<uniform> terrain_shadow: TerrainShadow;
@group(constants::MATERIAL_BIND_GROUP) @binding(121)
var terrain_shadow_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(122)
var terrain_shadow_sampler: sampler;

// `ocw_world::terrain_shadow::{MIN_SOFT_M, BIAS_M}`.
const MIN_SOFT_M: f32 = 3.0;
const BIAS_M: f32 = 3.0;

// How much of the sun a point sees past the mountains (0 … 1).
fn terrain_sun(p: vec3<f32>) -> f32 {
    if terrain_shadow.params.x < 0.5 {
        return 1.0;
    }
    let uv = p.xz * terrain_shadow.scale.xy + terrain_shadow.scale.zw;
    if uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0 {
        return 1.0;
    }
    let t = textureSampleLevel(terrain_shadow_map, terrain_shadow_sampler, uv, 0.0).xy;
    let line = t.x + terrain_shadow.params.y;
    let soft = max(t.y, MIN_SOFT_M);
    let v = smoothstep(line - soft, line + soft, p.y + BIAS_M);
    return mix(1.0, v, terrain_shadow.params.z);
}

// The sun's direct light on this fragment without any shadow (Bevy's own
// `directional_light`, the inputs as `apply_pbr_lighting` builds them).
fn sun_direct(pbr: PbrInput) -> vec3<f32> {
    let metallic = pbr.material.metallic;
    let pr = pbr.material.perceptual_roughness;
    let base = pbr.material.base_color.rgb;
    let NdotV = max(dot(pbr.N, pbr.V), 0.0001);
    var li: lighting::LightingInput;
    li.layers[LAYER_BASE].NdotV = NdotV;
    li.layers[LAYER_BASE].N = pbr.N;
    li.layers[LAYER_BASE].R = reflect(-pbr.V, pbr.N);
    li.layers[LAYER_BASE].perceptual_roughness = pr;
    li.layers[LAYER_BASE].roughness = lighting::perceptualRoughnessToRoughness(pr);
    li.P = pbr.world_position.xyz;
    li.V = pbr.V;
    li.diffuse_color = calculate_diffuse_color(
        base,
        metallic,
        pbr.material.specular_transmission,
        pbr.material.diffuse_transmission
    );
    li.metallic = metallic;
    li.F0_dielectric = calculate_F0_dielectric(pbr.material.reflectance);
    li.F0_metallic = base;
    li.F_ab = lighting::F_AB(pr, NdotV);
@if(STANDARD_MATERIAL_CLEARCOAT) {
    let ccn = pbr.clearcoat_N;
    let ccpr = pbr.material.clearcoat_perceptual_roughness;
    li.layers[LAYER_CLEARCOAT].NdotV = max(dot(ccn, pbr.V), 0.0001);
    li.layers[LAYER_CLEARCOAT].N = ccn;
    li.layers[LAYER_CLEARCOAT].R = reflect(-pbr.V, ccn);
    li.layers[LAYER_CLEARCOAT].perceptual_roughness = ccpr;
    li.layers[LAYER_CLEARCOAT].roughness = lighting::perceptualRoughnessToRoughness(ccpr);
    li.clearcoat_strength = pbr.material.clearcoat;
}
@if(STANDARD_MATERIAL_ANISOTROPY) {
    li.anisotropy = pbr.anisotropy_strength;
    li.Ta = pbr.anisotropy_T;
    li.Ba = pbr.anisotropy_B;
}
    return lighting::directional_light(0u, &li, true);
}

// `apply_pbr_lighting` with the terrain's shadow on the sun (the one
// directional light, `sky::Sun`).
fn lit(pbr: PbrInput) -> vec4<f32> {
    var input = pbr;
    let v = terrain_sun(input.world_position.xyz);
    let n = view_bindings::lights.n_directional_lights;
    var casts = false;
    if n > 0u {
        casts = (input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
            && (view_bindings::lights.directional_lights[0].flags
                & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u;
    }
    if casts {
        input.directional_shadow_factor = input.directional_shadow_factor * v;
    }
    var c = apply_pbr_lighting(input);
    if !casts && n > 0u && v < 0.999 {
        let direct = view_bindings::view.exposure * sun_direct(input);
        c = vec4<f32>(max(c.rgb - (1.0 - v) * direct, vec3<f32>(0.0)), c.a);
    }
    return c;
}
