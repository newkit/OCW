// Road vehicles (spec 14 Phase 18, 18E T7): the standard PBR fragment
// shader with each vehicle's paint and lamps from its instance tag, so all
// vehicles of a model and level of detail are one instanced draw.
//
// Vertex data (ocw-render `traffic::models`): uv.x = the vertex's role
// (`lamps::Role`: 0 fixed, 1 paint, 2 headlamp, 3 brake / tail lamp,
// 4 / 5 left / right indicator, 6 a bus's windows, 7 metal), uv.y = its
// perceptual roughness; the vertex colour is the slot's colour (paint:
// the model's shading only, the tag gives the colour).
//
// The tag (`lamps::tag`): bits 0-23 the paint (sRGB 0xRRGGBB), 24 dipped
// headlights, 25 daytime running lights, 26 brake, 27 / 28 left / right
// indicator lit, 29 interior lights. Rust twin of `lamp_level`:
// `lamps::lamp_level`.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, visibility_range_dither},
    mesh_bindings::mesh,
};

struct VehicleGlow {
    // The lamps' brightness (display-referred, by the scene's light):
    // x headlamps, y brake / tail, z indicators, w interior.
    level: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> vehicle_glow: VehicleGlow;

const TAG_HEAD: u32 = 16777216u;
const TAG_DRL: u32 = 33554432u;
const TAG_BRAKE: u32 = 67108864u;
const TAG_LEFT: u32 = 134217728u;
const TAG_RIGHT: u32 = 268435456u;
const TAG_INTERIOR: u32 = 536870912u;

fn has(tag: u32, bit: u32) -> bool {
    return (tag & bit) != 0u;
}

fn lamp_level(role: u32, tag: u32) -> f32 {
    if role == 2u {
        if has(tag, TAG_HEAD) {
            return 1.0;
        }
        if has(tag, TAG_DRL) {
            return 0.35;
        }
        return 0.0;
    }
    if role == 3u {
        if has(tag, TAG_BRAKE) {
            return 1.0;
        }
        if has(tag, TAG_HEAD) {
            return 0.3;
        }
        return 0.0;
    }
    if role == 4u {
        return select(0.0, 1.0, has(tag, TAG_LEFT));
    }
    if role == 5u {
        return select(0.0, 1.0, has(tag, TAG_RIGHT));
    }
    if role == 6u {
        return select(0.0, 1.0, has(tag, TAG_INTERIOR));
    }
    return 0.0;
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
@if(VISIBILITY_RANGE_DITHER)
    visibility_range_dither(in.position, in.visibility_range_dither);

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let tag = mesh[in.instance_index].tag;
    var role = 0u;
    var rough = 0.5;
@if(VERTEX_UVS_A) {
    role = u32(in.uv.x + 0.5);
    rough = in.uv.y;
}
    pbr_input.material.perceptual_roughness = clamp(rough, 0.04, 1.0);
    if role == 7u {
        pbr_input.material.metallic = 0.85;
    }
    if role == 1u {
        let paint = vec3<f32>(
            srgb_to_linear(f32((tag >> 16u) & 255u) / 255.0),
            srgb_to_linear(f32((tag >> 8u) & 255u) / 255.0),
            srgb_to_linear(f32(tag & 255u) / 255.0),
        );
        pbr_input.material.base_color = vec4<f32>(pbr_input.material.base_color.rgb * paint, 1.0);
        // A clear-coated car paint: a little gloss.
        pbr_input.material.metallic = 0.25;
    }
    let level = lamp_level(role, tag);
    if level > 0.0 {
        var colour = vec3<f32>(1.0, 0.95, 0.85) * vehicle_glow.level.x;
        if role == 3u {
            colour = vec3<f32>(1.0, 0.06, 0.03) * vehicle_glow.level.y;
        } else if role == 4u || role == 5u {
            colour = vec3<f32>(1.0, 0.45, 0.02) * vehicle_glow.level.z;
        } else if role == 6u {
            colour = vec3<f32>(1.0, 0.86, 0.6) * vehicle_glow.level.w;
        }
        // Display-referred (alpha 0: not scaled by the exposure).
        pbr_input.material.emissive = vec4<f32>(colour * level, 0.0);
    }

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
