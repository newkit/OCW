// Terrain with the tunnel tubes cut out: the standard PBR fragment shader,
// but terrain fragments inside a tube near the rider are discarded, so the
// hillside at a portal does not show inside the tube. `tunnel_cut_inside`
// mirrors `tunnel_cut::inside` in Rust.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
};

struct TunnelCut {
    // x: number of pieces; y: wall height; z: crown height (m).
    shape: vec4<f32>,
    // Two per piece: (centre xyz, half length), (direction xz, half width,
    // grade).
    pieces: array<vec4<f32>, 128>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> tunnel_cut: TunnelCut;

fn tunnel_cut_inside(p: vec3<f32>) -> bool {
    let n = u32(tunnel_cut.shape.x);
    let wall = tunnel_cut.shape.y;
    let crown = tunnel_cut.shape.z;
    for (var i = 0u; i < n; i = i + 1u) {
        let a = tunnel_cut.pieces[2u * i];
        let b = tunnel_cut.pieces[2u * i + 1u];
        let d = p - a.xyz;
        let along = dot(d, vec3<f32>(b.x, 0.0, b.y));
        if abs(along) > a.w {
            continue;
        }
        let lateral = abs(dot(d, vec3<f32>(-b.y, 0.0, b.x))) / b.z;
        let h = d.y - along * b.w;
        if lateral > 1.0 || h <= 0.0 {
            continue;
        }
        if h <= wall {
            return true;
        }
        let e = (h - wall) / (crown - wall);
        if lateral * lateral + e * e <= 1.0 {
            return true;
        }
    }
    return false;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    if tunnel_cut_inside(in.world_position.xyz) {
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
