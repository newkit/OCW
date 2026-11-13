// Terrain (spec §4.6, Phase 8 T2): the standard PBR fragment shader with
// the ground materials of the library blended by the tile's splat weights,
// and terrain fragments inside a tunnel tube near the rider discarded.
// `ground::blend_weights` and `tunnel_cut::inside` are the CPU twins.
// Independent of the light setup: only the PBR input is changed, the
// lighting is Bevy's.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::view,
};

struct TunnelCut {
    // x: number of pieces; y: wall height; z: crown height (m).
    shape: vec4<f32>,
    // Two per piece: (centre xyz, half length), (direction xz, half width,
    // grade).
    pieces: array<vec4<f32>, 128>,
}

struct TerrainLook {
    // x: textured; y: land-use cells per tile side; z, w: fade start, end (m).
    mode: vec4<f32>,
    // Repeat size (m) of the nine layers.
    sizes: array<vec4<f32>, 3>,
    // Near tint per layer (regional tint hook, neutral 1).
    material_tint: array<vec4<f32>, 9>,
    // x: normal strength; y: macro scale; z: blend contrast; w: border warp (m).
    params: vec4<f32>,
    // x: tile size (m).
    tile: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> tunnel_cut: TunnelCut;
@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var<uniform> look: TerrainLook;
@group(constants::MATERIAL_BIND_GROUP) @binding(102)
var splat_a: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(103)
var splat_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(104)
var splat_b: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(105)
var albedo_array: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(106)
var array_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(107)
var normal_array: texture_2d_array<f32>;

const ROCK: u32 = 3u;
const LAYERS: u32 = 9u;

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

fn hash2(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(0.1031, 0.1030));
    q = q + dot(q, q.yx + 33.33);
    return fract((q.x + q.y) * q.x);
}

// Value noise, 0 … 1.
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

fn layer_size(layer: u32) -> f32 {
    return look.sizes[layer / 4u][layer % 4u];
}

// The macro sample is rotated, so its repeats do not line up with the detail's.
fn rot(v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(0.8 * v.x - 0.6 * v.y, 0.6 * v.x + 0.8 * v.y);
}

fn rot_back(v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(0.8 * v.x + 0.6 * v.y, -0.6 * v.x + 0.8 * v.y);
}

struct Sample {
    albedo: vec4<f32>,
    // Tangent-space normal x, y (texture axes).
    n: vec2<f32>,
}

// A layer on a plane with coordinates `p` (m) and their screen derivatives,
// at two scales: detail and rotated macro; `far` (0 … 1) moves the weight
// to the macro scale.
fn sample_layer(layer: u32, p: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, far: f32) -> Sample {
    let s = layer_size(layer);
    let m = s * look.params.y;
    let uv1 = p / s;
    let uv2 = rot(p) / m + vec2<f32>(0.37, 0.71);
    let a1 = textureSampleGrad(albedo_array, array_sampler, uv1, i32(layer), dx / s, dy / s);
    let a2 = textureSampleGrad(albedo_array, array_sampler, uv2, i32(layer), rot(dx) / m, rot(dy) / m);
    let n1 = textureSampleGrad(normal_array, array_sampler, uv1, i32(layer), dx / s, dy / s).xy * 2.0 - 1.0;
    let n2 = rot_back(textureSampleGrad(normal_array, array_sampler, uv2, i32(layer), rot(dx) / m, rot(dy) / m).xy * 2.0 - 1.0);
    let k = mix(0.3, 0.8, far);
    // The macro sample mostly varies the brightness of the detail.
    let lum1 = dot(a1.rgb, vec3<f32>(0.3, 0.5, 0.2)) + 1e-3;
    let lum2 = dot(a2.rgb, vec3<f32>(0.3, 0.5, 0.2)) + 1e-3;
    let detail = a1.rgb * mix(1.0, clamp(lum2 / lum1, 0.6, 1.6), 0.5 * k);
    var out: Sample;
    out.albedo = vec4<f32>(mix(detail, a2.rgb, 0.5 * k), mix(a1.a, a2.a, k));
    out.n = mix(n1, n2, k);
    return out;
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

    let wp = in.world_position.xyz;
    let dist = distance(wp, view.world_position.xyz);
    // Derivatives in uniform control flow, for the sampling below.
    let dx = dpdx(wp);
    let dy = dpdy(wp);
    let fade_start = look.mode.z;
    let fade_end = look.mode.w;
    if look.mode.x > 0.5 && dist < fade_end {
        let N = normalize(in.world_normal);
        // Splat weights at the cell corners, the lookup warped by noise.
        let cells = look.mode.y;
        let warp = look.params.w / look.tile.x;
        let wofs = vec2<f32>(
            vnoise(wp.xz * 0.13) - 0.5,
            vnoise(wp.xz * 0.13 + vec2<f32>(31.7, 11.3)) - 0.5,
        ) * 2.0 * warp;
        let uv = clamp(in.uv + wofs, vec2<f32>(0.0), vec2<f32>(1.0));
        let suv = (uv * cells + 0.5) / (cells + 1.0);
        let wa = textureSampleLevel(splat_a, splat_sampler, suv, 0.0);
        let wb = textureSampleLevel(splat_b, splat_sampler, suv, 0.0);
        var w: array<f32, 9>;
        w[0] = wa.x; w[1] = wa.y; w[2] = wa.z; w[3] = wa.w;
        w[4] = wb.x; w[5] = wb.y; w[6] = wb.z; w[7] = wb.w;
        w[8] = max(0.0, 1.0 - (wa.x + wa.y + wa.z + wa.w + wb.x + wb.y + wb.z + wb.w));
        // Rock on steep slopes (`ground::rock_weight`).
        let ny = max(N.y, 1e-3);
        let slope = sqrt(max(0.0, 1.0 - ny * ny)) / ny;
        let r = smoothstep(0.6, 1.0, slope);
        for (var i = 0u; i < LAYERS; i = i + 1u) {
            w[i] = w[i] * (1.0 - r);
        }
        w[ROCK] = w[ROCK] + r;
        // Height blend with a noise per material (`ground::blend_weights`).
        let contrast = look.params.z;
        var h: array<f32, 9>;
        var top = -1.0;
        for (var i = 0u; i < LAYERS; i = i + 1u) {
            let o = vec2<f32>(f32(i) * 17.3, f32(i) * 9.1);
            let nz = vnoise(wp.xz / 4.0 + o) * 0.6 + vnoise(wp.xz / 1.3 + o) * 0.4;
            h[i] = select(-1.0, w[i] + nz * 0.5, w[i] > 0.0);
            top = max(top, h[i]);
        }
        var sum = 0.0;
        for (var i = 0u; i < LAYERS; i = i + 1u) {
            w[i] = max(h[i] - (top - contrast), 0.0) * max(w[i], 0.0);
            sum = sum + w[i];
        }
        let far = smoothstep(15.0, fade_end, dist);
        var albedo = vec3<f32>(0.0);
        var rough = 0.0;
        var n = vec2<f32>(0.0);
        var tri_n = vec3<f32>(0.0);
        for (var i = 0u; i < LAYERS; i = i + 1u) {
            let wi = w[i] / max(sum, 1e-4);
            if wi < 0.02 {
                continue;
            }
            var s: Sample;
            if i == ROCK && slope > 0.45 {
                // Triplanar on steep ground: no stretched texture on cliffs.
                var b = pow(abs(N), vec3<f32>(4.0));
                b = b / (b.x + b.y + b.z);
                let sy = sample_layer(i, wp.xz, dx.xz, dy.xz, far);
                let sx = sample_layer(i, vec2<f32>(wp.z, -wp.y), vec2<f32>(dx.z, -dx.y), vec2<f32>(dy.z, -dy.y), far);
                let sz = sample_layer(i, vec2<f32>(wp.x, -wp.y), vec2<f32>(dx.x, -dx.y), vec2<f32>(dy.x, -dy.y), far);
                s.albedo = sy.albedo * b.y + sx.albedo * b.x + sz.albedo * b.z;
                s.n = sy.n * b.y;
                tri_n = tri_n + wi * (vec3<f32>(0.0, sx.n.y, sx.n.x) * b.x + vec3<f32>(sz.n.x, sz.n.y, 0.0) * b.z);
            } else {
                s = sample_layer(i, wp.xz, dx.xz, dy.xz, far);
            }
            albedo = albedo + wi * s.albedo.rgb * look.material_tint[i].rgb;
            rough = rough + wi * s.albedo.a;
            n = n + wi * s.n;
        }
        // Top-down projection: texture u = +x, image up (GL green) = -z.
        let strength = look.params.x;
        let nw = normalize(N + strength * (vec3<f32>(n.x, 0.0, -n.y) + tri_n));
        let t = smoothstep(fade_start, fade_end, dist);
        pbr_input.material.base_color = vec4<f32>(mix(albedo, pbr_input.material.base_color.rgb, t), 1.0);
        pbr_input.material.perceptual_roughness = mix(clamp(rough, 0.3, 1.0), pbr_input.material.perceptual_roughness, t);
        pbr_input.N = normalize(mix(nw, pbr_input.N, t));
    }

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
